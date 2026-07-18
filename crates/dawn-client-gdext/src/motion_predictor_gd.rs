use dawn_client_core::{MotionPredictor as CoreMotionPredictor, MotionProfile};
use godot::prelude::*;

/// Godot adapter for the Rust client-side motion predictor (ADR-0043).
///
/// The adapter owns no movement policy. It only translates Godot `Vector3`
/// values to the plain arrays used by `dawn-client-core`.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct MotionPredictor {
    core: CoreMotionPredictor,
}

#[godot_api]
impl MotionPredictor {
    /// Seed the predictor from the server-provided ship profile and state.
    #[func]
    fn configure(
        &mut self,
        max_speed: f64,
        mass: f64,
        inertia_modifier: f64,
        position: Vector3,
        velocity: Vector3,
        tick: i64,
    ) {
        let profile = MotionProfile::new(max_speed, mass, inertia_modifier).unwrap_or_default();
        self.core.configure(
            profile,
            to_array(position),
            to_array(velocity),
            tick.max(0) as u64,
        );
    }

    /// Configure a remote ship's shared motion track.
    #[func]
    fn configure_dead_reckoning(
        &mut self,
        max_speed: f64,
        mass: f64,
        inertia_modifier: f64,
        position: Vector3,
        velocity: Vector3,
        tick: i64,
    ) {
        let profile = MotionProfile::new(max_speed, mass, inertia_modifier).unwrap_or_default();
        self.core.configure_dead_reckoning(
            profile,
            to_array(position),
            to_array(velocity),
            tick.max(0) as u64,
        );
    }

    #[func]
    fn enable_prediction(&mut self) {
        self.core.enable_prediction();
    }

    #[func]
    fn enable_dead_reckoning(&mut self) {
        self.core.enable_dead_reckoning();
    }

    #[func]
    fn set_thrust_direction(&mut self, direction: Vector3) {
        self.core.set_thrust(to_array(direction));
    }

    #[func]
    fn set_braking(&mut self) {
        self.core.set_braking();
    }

    #[func]
    fn clear_input(&mut self) {
        self.core.clear_input();
    }

    #[func]
    fn set_velocity(&mut self, velocity: Vector3) {
        self.core.set_velocity(to_array(velocity));
    }

    #[func]
    fn advance(&mut self, ticks: f64) {
        self.core.advance(ticks);
    }

    /// Returns false when a stale server correction was ignored.
    #[func]
    fn reconcile(&mut self, position: Vector3, velocity: Vector3, tick: i64) -> bool {
        self.core
            .reconcile(to_array(position), to_array(velocity), tick.max(0) as u64)
    }

    #[func]
    fn reset(&mut self, position: Vector3, velocity: Vector3, tick: i64) {
        self.core
            .reset(to_array(position), to_array(velocity), tick.max(0) as u64);
    }

    #[func]
    fn rebase(&mut self, shift: Vector3) {
        self.core.rebase(to_array(shift));
    }

    #[func]
    fn predicted_position(&self) -> Vector3 {
        from_array(self.core.predicted_position())
    }

    #[func]
    fn predicted_velocity(&self) -> Vector3 {
        from_array(self.core.velocity())
    }
}

fn to_array(vector: Vector3) -> [f64; 3] {
    [vector.x as f64, vector.y as f64, vector.z as f64]
}

fn from_array(vector: [f64; 3]) -> Vector3 {
    Vector3::new(vector[0] as f32, vector[1] as f32, vector[2] as f32)
}
