//! Command-driven client motion for one ship.
//!
//! [`ShipMotion`] composes the existing fixed-tick predictor with the
//! floating-origin coordinate policy. Callers submit server-space commands
//! and consume a [`MotionFrame`]; neither the caller nor the Godot adapter
//! needs to coordinate prediction, correction, and coordinate conversion.

use crate::{MotionInput, MotionPredictor, MotionProfile, MotionState, WorldSpace};

// Preserve the existing 2,000 render units/tick presentation threshold.
const RENDER_SPEED_CAP: f64 = 2_000.0;

/// A state change or clock update accepted by one ship's motion track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MotionCommand {
    /// Apply a local input in server-space axes.
    SetInput(MotionInput),
    /// Clear local steering and return to inertial flight.
    ClearInput,
    /// Advance the track by a fractional number of server ticks.
    Advance { ticks: f64 },
    /// Apply a velocity-only authoritative event.
    SetVelocity { velocity: [f64; 3], tick: u64 },
    /// Apply a position and velocity correction from the server.
    AuthoritativeSample {
        position: [f64; 3],
        velocity: [f64; 3],
        tick: u64,
    },
    /// Reset at an authoritative discontinuity such as warp arrival.
    Reset {
        position: [f64; 3],
        velocity: [f64; 3],
        tick: u64,
    },
    /// Enter the docked state at an authoritative position.
    Dock { position: [f64; 3], tick: u64 },
    /// Leave the docked state using the track's current control role.
    Undock {
        position: [f64; 3],
        velocity: [f64; 3],
        tick: u64,
    },
    /// Notify a change of piloted Ship; the module chooses its motion mode.
    SetLocallyControlled(bool),
    /// Move the floating origin while preserving absolute server coordinates.
    Rebase { new_origin: [f64; 3] },
}

/// Result of dispatching a motion command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MotionDispatch {
    /// The command changed or advanced the track.
    Applied,
    /// The command was rejected as stale or invalid.
    Ignored,
    /// The origin changed and the value is the render-space shift.
    Rebased { render_shift: [f64; 3] },
}

impl MotionDispatch {
    /// Returns whether the command was accepted.
    pub const fn accepted(self) -> bool {
        !matches!(self, Self::Ignored)
    }
}

/// The complete result consumed by a presentation adapter for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionFrame {
    /// Last authoritative position in server-space metres.
    pub authoritative_position: [f64; 3],
    /// Current predicted/presented position in server-space metres.
    pub predicted_position: [f64; 3],
    /// Origin-relative position in render-space units.
    pub render_position: [f64; 3],
    /// Velocity in server-space units per tick.
    pub server_velocity: [f64; 3],
    /// Velocity in render-space units per tick.
    pub render_velocity: [f64; 3],
    /// Current presentation state.
    pub state: MotionState,
    /// Last logical tick applied to the track.
    pub tick: u64,
    /// Whether the Ship mesh should be visible (remote high-speed Ships hide).
    pub visible: bool,
}

/// One ship's command-driven motion state and coordinate policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShipMotion {
    track: MotionPredictor,
    world: WorldSpace,
    locally_controlled: bool,
}

impl Default for ShipMotion {
    fn default() -> Self {
        Self::new(MotionProfile::default(), [0.0; 3], [0.0; 3], 0)
            .expect("the default motion state is valid")
    }
}

impl ShipMotion {
    /// Creates a local-prediction track at an absolute server-space position.
    pub fn new(
        profile: MotionProfile,
        position: [f64; 3],
        velocity: [f64; 3],
        tick: u64,
    ) -> Option<Self> {
        (vector_is_finite(position) && vector_is_finite(velocity)).then_some(Self {
            track: MotionPredictor::new(profile, position, velocity, tick),
            world: WorldSpace::new(),
            locally_controlled: true,
        })
    }

    /// Seeds a remote ship and clears any previous command history.
    pub fn configure_dead_reckoning(
        &mut self,
        profile: MotionProfile,
        position: [f64; 3],
        velocity: [f64; 3],
        tick: u64,
    ) -> bool {
        if !vector_is_finite(position) || !vector_is_finite(velocity) {
            return false;
        }
        self.track
            .configure_dead_reckoning(profile, position, velocity, tick);
        self.locally_controlled = false;
        true
    }

    /// Dispatches a command in server-space units.
    pub fn dispatch(&mut self, command: MotionCommand) -> MotionDispatch {
        match command {
            MotionCommand::SetInput(input) if self.locally_controlled && input_is_finite(input) => {
                match input {
                    MotionInput::Coast => self.track.clear_input(),
                    MotionInput::Thrust(direction) => self.track.set_thrust(direction),
                    MotionInput::Brake => self.track.set_braking(),
                }
                MotionDispatch::Applied
            }
            MotionCommand::SetInput(_) => MotionDispatch::Ignored,
            MotionCommand::ClearInput => {
                self.track.clear_input();
                MotionDispatch::Applied
            }
            MotionCommand::Advance { ticks } if ticks.is_finite() && ticks > 0.0 => {
                self.track.advance(ticks);
                MotionDispatch::Applied
            }
            MotionCommand::Advance { .. } => MotionDispatch::Ignored,
            MotionCommand::SetVelocity { velocity, tick } if vector_is_finite(velocity) => {
                if self.track.set_velocity_at_tick(velocity, tick) {
                    if self.locally_controlled && self.server_speed() > self.server_speed_cap() {
                        self.track.begin_warp(self.server_speed_cap());
                    }
                    MotionDispatch::Applied
                } else {
                    MotionDispatch::Ignored
                }
            }
            MotionCommand::SetVelocity { .. } => MotionDispatch::Ignored,
            MotionCommand::AuthoritativeSample {
                position,
                velocity,
                tick,
            } if vector_is_finite(position) && vector_is_finite(velocity) => {
                if self.track.reconcile(position, velocity, tick) {
                    MotionDispatch::Applied
                } else {
                    MotionDispatch::Ignored
                }
            }
            MotionCommand::AuthoritativeSample { .. } => MotionDispatch::Ignored,
            MotionCommand::Reset {
                position,
                velocity,
                tick,
            } if vector_is_finite(position) && vector_is_finite(velocity) => {
                self.track.reset(position, velocity, tick);
                self.select_normal_mode();
                MotionDispatch::Applied
            }
            MotionCommand::Reset { .. } => MotionDispatch::Ignored,
            MotionCommand::Dock { position, tick } if vector_is_finite(position) => {
                if self.track.dock(position, tick) {
                    MotionDispatch::Applied
                } else {
                    MotionDispatch::Ignored
                }
            }
            MotionCommand::Dock { .. } => MotionDispatch::Ignored,
            MotionCommand::Undock {
                position,
                velocity,
                tick,
            } if vector_is_finite(position) && vector_is_finite(velocity) => {
                if self
                    .track
                    .undock(position, velocity, tick, self.locally_controlled)
                {
                    MotionDispatch::Applied
                } else {
                    MotionDispatch::Ignored
                }
            }
            MotionCommand::Undock { .. } => MotionDispatch::Ignored,
            MotionCommand::SetLocallyControlled(controlled) => {
                self.locally_controlled = controlled;
                if self.track.state() != MotionState::Docked {
                    if controlled
                        && (self.track.state() == MotionState::WarpPresentation
                            || self.server_speed() > self.server_speed_cap())
                    {
                        self.track.begin_warp(self.server_speed_cap());
                    } else {
                        self.select_normal_mode();
                    }
                }
                MotionDispatch::Applied
            }
            MotionCommand::Rebase { new_origin } if vector_is_finite(new_origin) => {
                MotionDispatch::Rebased {
                    render_shift: self.world.rebase_to(new_origin),
                }
            }
            MotionCommand::Rebase { .. } => MotionDispatch::Ignored,
        }
    }

    /// Returns a server-space and render-space snapshot without advancing it.
    pub fn frame(&self) -> MotionFrame {
        let authoritative_position = self.track.position();
        let predicted_position = self.track.predicted_position();
        let server_velocity = self.track.velocity();
        MotionFrame {
            authoritative_position,
            predicted_position,
            render_position: self.world.server_to_render(predicted_position),
            server_velocity,
            render_velocity: self.world.server_direction_to_render(server_velocity),
            state: self.track.state(),
            tick: self.track.tick(),
            visible: self.locally_controlled || self.server_speed() <= self.server_speed_cap(),
        }
    }

    /// Returns the continuous server-space observer position for world effects.
    ///
    /// During warp this advances at the reported velocity without the ship
    /// mesh's speed cap. Direction-only effects can therefore retain parallax
    /// while [`MotionFrame::render_position`] remains bounded.
    pub fn world_presentation_position(&self) -> [f64; 3] {
        self.track.world_presentation_position()
    }

    /// Returns the current server-space speed.
    pub fn server_speed(&self) -> f64 {
        magnitude(self.track.velocity())
    }

    /// Returns the current render-space speed.
    pub fn render_speed(&self) -> f64 {
        magnitude(self.frame().render_velocity)
    }

    /// Converts a render-space direction into the server-space axes consumed
    /// by the movement policy.
    pub fn render_direction_to_server(&self, direction: [f64; 3]) -> [f64; 3] {
        self.world.render_direction_to_server(direction)
    }

    fn server_speed_cap(&self) -> f64 {
        self.world
            .render_speed_to_server(RENDER_SPEED_CAP)
            .expect("the fixed positive render speed cap is valid")
    }

    fn select_normal_mode(&mut self) {
        if self.locally_controlled {
            self.track.enable_prediction();
        } else {
            self.track.enable_dead_reckoning();
        }
    }
}

fn input_is_finite(input: MotionInput) -> bool {
    match input {
        MotionInput::Coast | MotionInput::Brake => true,
        MotionInput::Thrust(direction) => vector_is_finite(direction),
    }
}

fn vector_is_finite(vector: [f64; 3]) -> bool {
    vector.iter().all(|component| component.is_finite())
}

fn magnitude(vector: [f64; 3]) -> f64 {
    vector
        .iter()
        .map(|component| component * component)
        .sum::<f64>()
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::{MotionCommand, MotionDispatch, MotionState, ShipMotion};
    use crate::{MotionInput, MotionProfile, WORLD_SCALE};

    #[test]
    fn local_velocity_selects_warp_and_deceleration_retains_it_until_reset() {
        let mut motion = ShipMotion::default();
        motion.dispatch(MotionCommand::SetVelocity {
            velocity: [30_000.0, 0.0, 0.0],
            tick: 0,
        });
        motion.dispatch(MotionCommand::Advance { ticks: 1.0 });
        assert_eq!(motion.frame().state, MotionState::WarpPresentation);
        assert_eq!(motion.frame().render_position, [2_000.0, 0.0, 0.0]);
        motion.dispatch(MotionCommand::SetVelocity {
            velocity: [10_000.0, 0.0, 0.0],
            tick: 1,
        });
        motion.dispatch(MotionCommand::Advance { ticks: 0.1 });
        assert_eq!(motion.frame().state, MotionState::WarpPresentation);
        assert!((motion.frame().render_position[0] - (2_000.0 + 200.0 / 3.0)).abs() < 1e-9);
        motion.dispatch(MotionCommand::Reset {
            position: [70.0, 0.0, 0.0],
            velocity: [0.0; 3],
            tick: 12,
        });
        assert_eq!(motion.frame().state, MotionState::Prediction);
        assert_eq!(motion.frame().render_position, [7.0, 0.0, 0.0]);
    }

    #[test]
    fn taking_control_of_a_warping_remote_ship_selects_the_bounded_track() {
        let mut motion = ShipMotion::default();
        assert!(motion.configure_dead_reckoning(
            MotionProfile::default(),
            [0.0; 3],
            [30_000.0, 0.0, 0.0],
            0
        ));
        assert_eq!(motion.frame().state, MotionState::DeadReckoning);
        assert!(!motion.frame().visible);
        assert_eq!(
            motion.dispatch(MotionCommand::SetInput(MotionInput::Brake)),
            MotionDispatch::Ignored
        );
        motion.dispatch(MotionCommand::SetLocallyControlled(true));
        assert!(motion.frame().visible);
        motion.dispatch(MotionCommand::Advance { ticks: 1.0 });
        assert_eq!(motion.frame().render_position, [2_000.0, 0.0, 0.0]);
        motion.dispatch(MotionCommand::SetVelocity {
            velocity: [10_000.0, 0.0, 0.0],
            tick: 1,
        });
        motion.dispatch(MotionCommand::SetLocallyControlled(true));
        assert_eq!(motion.frame().state, MotionState::WarpPresentation);
        motion.dispatch(MotionCommand::SetLocallyControlled(false));
        assert_eq!(motion.frame().state, MotionState::DeadReckoning);
        assert!(motion.frame().visible);
        motion.dispatch(MotionCommand::SetLocallyControlled(true));
        assert_eq!(motion.frame().state, MotionState::Prediction);
    }

    #[test]
    fn docked_role_changes_do_not_undock_and_undock_uses_the_latest_role() {
        for controlled in [false, true] {
            let mut motion = ShipMotion::default();
            motion.dispatch(MotionCommand::Dock {
                position: [50.0, 0.0, 0.0],
                tick: 5,
            });
            motion.dispatch(MotionCommand::SetLocallyControlled(controlled));
            motion.dispatch(MotionCommand::Advance { ticks: 2.0 });
            assert_eq!(motion.frame().state, MotionState::Docked);
            assert_eq!(motion.frame().server_velocity, [0.0; 3]);
            assert_eq!(motion.frame().predicted_position, [50.0, 0.0, 0.0]);
            let before = motion;
            assert_eq!(
                motion.dispatch(MotionCommand::Undock {
                    position: [0.0; 3],
                    velocity: [1.0; 3],
                    tick: 4
                }),
                MotionDispatch::Ignored
            );
            assert_eq!(motion, before);
            motion.dispatch(MotionCommand::Undock {
                position: [60.0, 0.0, 0.0],
                velocity: [1.0, 0.0, 0.0],
                tick: 6,
            });
            let expected = if controlled {
                MotionState::Prediction
            } else {
                MotionState::DeadReckoning
            };
            assert_eq!(motion.frame().state, expected);
            motion.dispatch(MotionCommand::Reset {
                position: [70.0, 0.0, 0.0],
                velocity: [0.0; 3],
                tick: 7,
            });
            assert_eq!(motion.frame().state, expected);
        }
    }

    #[test]
    fn threshold_and_stale_velocity_do_not_spuriously_change_modes() {
        let mut motion = ShipMotion::default();
        motion.dispatch(MotionCommand::SetVelocity {
            velocity: [20_000.0, 0.0, 0.0],
            tick: 5,
        });
        assert_eq!(motion.frame().state, MotionState::Prediction);
        let before = motion;
        for velocity in [[30_000.0, 0.0, 0.0], [f64::NAN, 0.0, 0.0]] {
            assert_eq!(
                motion.dispatch(MotionCommand::SetVelocity { velocity, tick: 4 }),
                MotionDispatch::Ignored
            );
            assert_eq!(motion, before);
        }
        motion.dispatch(MotionCommand::SetLocallyControlled(false));
        assert!(motion.frame().visible);
        motion.dispatch(MotionCommand::SetVelocity {
            velocity: [20_001.0, 0.0, 0.0],
            tick: 6,
        });
        assert!(!motion.frame().visible);
        assert_eq!(motion.frame().state, MotionState::DeadReckoning);
    }

    #[test]
    fn frame_keeps_absolute_f64_state_and_converts_only_at_render_boundary() {
        let absolute = [5.0e12 + 25.0, -4.0, 8.0];
        let mut motion = ShipMotion::new(MotionProfile::default(), absolute, [2.0, -4.0, 8.0], 7)
            .expect("test coordinates are finite");

        let before = motion.frame();
        assert_eq!(before.authoritative_position, absolute);
        assert_eq!(before.predicted_position, absolute);
        assert_eq!(
            before.render_position,
            [absolute[0] * WORLD_SCALE, -0.4, -0.8]
        );

        let rebase = motion.dispatch(MotionCommand::Rebase {
            new_origin: [5.0e12, 0.0, 0.0],
        });
        assert!(matches!(rebase, MotionDispatch::Rebased { .. }));

        let after = motion.frame();
        assert_eq!(after.authoritative_position, absolute);
        assert_eq!(after.predicted_position, absolute);
        assert_eq!(after.render_position, [2.5, -0.4, -0.8]);
        assert_eq!(after.server_velocity, [2.0, -4.0, 8.0]);
    }

    #[test]
    fn command_dispatch_owns_tick_advance_and_returns_a_typed_frame() {
        let mut motion = ShipMotion::default();
        assert_eq!(
            motion.dispatch(MotionCommand::SetInput(MotionInput::Thrust([
                1.0, 0.0, 0.0,
            ]))),
            MotionDispatch::Applied
        );
        assert_eq!(
            motion.dispatch(MotionCommand::Advance { ticks: 1.0 }),
            MotionDispatch::Applied
        );

        let frame = motion.frame();
        assert_eq!(frame.tick, 1);
        assert_eq!(frame.state, MotionState::Prediction);
        assert!(frame.server_velocity[0] > 0.0);
    }

    #[test]
    fn stale_authoritative_commands_do_not_change_the_frame() {
        let mut motion = ShipMotion::default();
        motion.dispatch(MotionCommand::AuthoritativeSample {
            position: [10.0, 0.0, 0.0],
            velocity: [1.0, 0.0, 0.0],
            tick: 10,
        });
        let before = motion.frame();

        assert_eq!(
            motion.dispatch(MotionCommand::AuthoritativeSample {
                position: [1.0, 0.0, 0.0],
                velocity: [9.0, 0.0, 0.0],
                tick: 9,
            }),
            MotionDispatch::Ignored
        );
        assert_eq!(motion.frame(), before);
    }

    #[test]
    fn invalid_commands_are_rejected_before_the_track_sees_them() {
        let mut motion = ShipMotion::default();
        assert_eq!(
            motion.dispatch(MotionCommand::SetInput(MotionInput::Thrust([
                f64::NAN,
                0.0,
                0.0,
            ]))),
            MotionDispatch::Ignored
        );
        assert_eq!(
            motion.dispatch(MotionCommand::Advance { ticks: f64::NAN }),
            MotionDispatch::Ignored
        );
        assert_eq!(motion.frame().server_velocity, [0.0; 3]);
    }

    #[test]
    fn world_presentation_position_remains_continuous_during_warp() {
        let mut motion =
            ShipMotion::new(MotionProfile::default(), [0.0; 3], [30_000.0, 0.0, 0.0], 0)
                .expect("test motion is valid");

        assert_eq!(
            motion.dispatch(MotionCommand::SetLocallyControlled(true)),
            MotionDispatch::Applied
        );
        assert_eq!(
            motion.dispatch(MotionCommand::Advance { ticks: 0.5 }),
            MotionDispatch::Applied
        );

        assert_eq!(motion.frame().predicted_position, [10_000.0, 0.0, 0.0]);
        assert_eq!(motion.world_presentation_position(), [15_000.0, 0.0, 0.0]);

        assert_eq!(
            motion.dispatch(MotionCommand::SetVelocity {
                velocity: [40_000.0, 0.0, 0.0],
                tick: 1,
            }),
            MotionDispatch::Applied
        );
        assert_eq!(motion.world_presentation_position(), [15_000.0, 0.0, 0.0]);

        motion.dispatch(MotionCommand::Advance { ticks: 0.5 });
        assert_eq!(motion.frame().predicted_position, [20_000.0, 0.0, 0.0]);
        assert_eq!(motion.world_presentation_position(), [35_000.0, 0.0, 0.0]);
    }

    #[test]
    fn warp_world_observer_accepts_corrections_without_moving_the_bounded_mesh() {
        let mut motion =
            ShipMotion::new(MotionProfile::default(), [0.0; 3], [1.0e9, 0.0, 0.0], 0).unwrap();
        motion.dispatch(MotionCommand::SetLocallyControlled(true));
        motion.dispatch(MotionCommand::Advance { ticks: 1.2 });
        let mesh = motion.frame().predicted_position;
        let sample = MotionCommand::AuthoritativeSample {
            position: [1.0e9, 0.0, 0.0],
            velocity: [10.0, 0.0, 0.0],
            tick: 1,
        };
        assert_eq!(motion.dispatch(sample), MotionDispatch::Applied);
        assert_eq!(motion.world_presentation_position(), [1.0e9, 0.0, 0.0]);
        assert_eq!(motion.frame().predicted_position, mesh);
        motion.dispatch(MotionCommand::Advance { ticks: 0.5 });
        assert_eq!(
            motion.world_presentation_position(),
            [1.0e9 + 5.0, 0.0, 0.0]
        );
        let before = motion.world_presentation_position();
        assert_eq!(
            motion.dispatch(MotionCommand::AuthoritativeSample {
                position: [0.0; 3],
                velocity: [0.0; 3],
                tick: 0,
            }),
            MotionDispatch::Ignored
        );
        assert_eq!(motion.world_presentation_position(), before);
    }
}
