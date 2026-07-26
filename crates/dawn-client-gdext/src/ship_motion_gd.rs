use dawn_client_core::{MotionCommand, MotionInput, MotionProfile, ShipMotion as CoreShipMotion};
use godot::prelude::*;

/// Godot adapter for the command-driven Rust motion surface.
///
/// Server-space positions enter as `PackedFloat64Array` so large absolute
/// coordinates are not narrowed before the core subtracts the floating
/// origin. Only the final render frame is converted to `Vector3` here.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct ShipMotion {
    core: CoreShipMotion,
}

#[godot_api]
impl ShipMotion {
    #[func]
    fn configure_dead_reckoning(
        &mut self,
        max_speed: f64,
        mass: f64,
        inertia_modifier: f64,
        server_position: PackedFloat64Array,
        server_velocity: Vector3,
        tick: i64,
    ) -> bool {
        let Some(position) = to_components(server_position) else {
            return false;
        };
        let Some(profile) = MotionProfile::new(max_speed, mass, inertia_modifier) else {
            return false;
        };
        self.core.configure_dead_reckoning(
            profile,
            position,
            to_array(server_velocity),
            tick.max(0) as u64,
        )
    }

    #[func]
    fn enable_prediction(&mut self) {
        self.core.dispatch(MotionCommand::EnablePrediction);
    }

    #[func]
    fn enable_dead_reckoning(&mut self) {
        self.core.dispatch(MotionCommand::EnableDeadReckoning);
    }

    #[func]
    fn begin_warp(&mut self, render_speed_cap: f64) -> bool {
        let Some(server_speed_cap) = self.core.render_speed_to_server(render_speed_cap) else {
            return false;
        };
        self.core
            .dispatch(MotionCommand::BeginWarp { server_speed_cap })
            .accepted()
    }

    #[func]
    fn set_thrust_direction(&mut self, render_direction: Vector3) -> bool {
        let direction = self
            .core
            .render_direction_to_server(to_array(render_direction));
        self.core
            .dispatch(MotionCommand::SetInput(MotionInput::Thrust(direction)))
            .accepted()
    }

    #[func]
    fn set_braking(&mut self) -> bool {
        self.core
            .dispatch(MotionCommand::SetInput(MotionInput::Brake))
            .accepted()
    }

    #[func]
    fn clear_input(&mut self) -> bool {
        self.core.dispatch(MotionCommand::ClearInput).accepted()
    }

    #[func]
    fn set_velocity(&mut self, server_velocity: Vector3, tick: i64) -> bool {
        self.core
            .dispatch(MotionCommand::SetVelocity {
                velocity: to_array(server_velocity),
                tick: tick.max(0) as u64,
            })
            .accepted()
    }

    #[func]
    fn advance(&mut self, ticks: f64) -> bool {
        self.core
            .dispatch(MotionCommand::Advance { ticks })
            .accepted()
    }

    #[func]
    fn reconcile(
        &mut self,
        server_position: PackedFloat64Array,
        server_velocity: Vector3,
        tick: i64,
    ) -> bool {
        let Some(position) = to_components(server_position) else {
            return false;
        };
        self.core
            .dispatch(MotionCommand::AuthoritativeSample {
                position,
                velocity: to_array(server_velocity),
                tick: tick.max(0) as u64,
            })
            .accepted()
    }

    #[func]
    fn reset(
        &mut self,
        server_position: PackedFloat64Array,
        server_velocity: Vector3,
        tick: i64,
        predict_locally: bool,
    ) -> bool {
        let Some(position) = to_components(server_position) else {
            return false;
        };
        let accepted = self
            .core
            .dispatch(MotionCommand::Reset {
                position,
                velocity: to_array(server_velocity),
                tick: tick.max(0) as u64,
            })
            .accepted();
        if !accepted {
            return false;
        }
        self.core.dispatch(if predict_locally {
            MotionCommand::EnablePrediction
        } else {
            MotionCommand::EnableDeadReckoning
        });
        true
    }

    #[func]
    fn dock(&mut self, server_position: PackedFloat64Array, tick: i64) -> bool {
        let Some(position) = to_components(server_position) else {
            return false;
        };
        self.core
            .dispatch(MotionCommand::Dock {
                position,
                tick: tick.max(0) as u64,
            })
            .accepted()
    }

    #[func]
    fn undock(
        &mut self,
        server_position: PackedFloat64Array,
        server_velocity: Vector3,
        tick: i64,
        predict_locally: bool,
    ) -> bool {
        let Some(position) = to_components(server_position) else {
            return false;
        };
        self.core
            .dispatch(MotionCommand::Undock {
                position,
                velocity: to_array(server_velocity),
                tick: tick.max(0) as u64,
                predict_locally,
            })
            .accepted()
    }

    #[func]
    fn rebase_to_components(&mut self, new_x: f64, new_y: f64, new_z: f64) -> Vector3 {
        let result = self.core.dispatch(MotionCommand::Rebase {
            new_origin: [new_x, new_y, new_z],
        });
        match result {
            dawn_client_core::MotionDispatch::Rebased { render_shift } => to_vector3(render_shift),
            _ => Vector3::ZERO,
        }
    }

    #[func]
    fn render_position(&self) -> Vector3 {
        to_vector3(self.core.frame().render_position)
    }

    #[func]
    fn render_velocity(&self) -> Vector3 {
        to_vector3(self.core.frame().render_velocity)
    }

    #[func]
    fn server_position(&self) -> PackedFloat64Array {
        self.core.frame().predicted_position.into()
    }

    #[func]
    fn authoritative_position(&self) -> PackedFloat64Array {
        self.core.frame().authoritative_position.into()
    }

    #[func]
    fn server_speed(&self) -> f64 {
        self.core.server_speed()
    }

    #[func]
    fn render_speed(&self) -> f64 {
        self.core.render_speed()
    }
}

fn to_array(vector: Vector3) -> [f64; 3] {
    [vector.x as f64, vector.y as f64, vector.z as f64]
}

fn to_components(array: PackedFloat64Array) -> Option<[f64; 3]> {
    (array.len() == 3).then(|| [array[0], array[1], array[2]])
}

fn to_vector3(vector: [f64; 3]) -> Vector3 {
    Vector3::new(vector[0] as f32, vector[1] as f32, vector[2] as f32)
}
