use dawn_client_core::WorldSpace as CoreWorldSpace;
use godot::prelude::*;

/// Godot adapter for the client-core floating-origin coordinate model.
///
/// The core keeps all absolute and origin-relative arithmetic in `f64`; this
/// adapter only converts the final render values to Godot's `Vector3`.
#[derive(Debug, GodotClass)]
#[class(init, base=RefCounted)]
pub struct WorldSpace {
    core: CoreWorldSpace,
}

#[godot_api]
impl WorldSpace {
    #[func]
    fn to_godot(&self, server_position: Vector3) -> Vector3 {
        self.to_godot_components(
            f64::from(server_position.x),
            f64::from(server_position.y),
            f64::from(server_position.z),
        )
    }

    #[func]
    fn to_godot_components(&self, server_x: f64, server_y: f64, server_z: f64) -> Vector3 {
        to_vector3(self.core.server_to_render([server_x, server_y, server_z]))
    }

    #[func]
    fn to_server(&self, godot_position: Vector3) -> Vector3 {
        to_vector3(self.core.render_to_server(to_array(godot_position)))
    }

    #[func]
    fn to_server_components(&self, godot_position: Vector3) -> PackedFloat64Array {
        self.core.render_to_server(to_array(godot_position)).into()
    }

    #[func]
    fn dir_to_godot(&self, server_direction: Vector3) -> Vector3 {
        to_vector3(
            self.core
                .server_direction_to_render(to_array(server_direction)),
        )
    }

    #[func]
    fn dir_to_server(&self, godot_direction: Vector3) -> Vector3 {
        to_vector3(
            self.core
                .render_direction_to_server(to_array(godot_direction)),
        )
    }

    #[func]
    fn should_rebase(&self, player_server_position: Vector3) -> bool {
        self.core.should_rebase(to_array(player_server_position))
    }

    #[func]
    fn should_rebase_components(&self, player_x: f64, player_y: f64, player_z: f64) -> bool {
        self.core.should_rebase([player_x, player_y, player_z])
    }

    #[func]
    fn distance_components(&self, first: PackedFloat64Array, second: PackedFloat64Array) -> f64 {
        let Some(first) = to_components(first) else {
            return f64::NAN;
        };
        let Some(second) = to_components(second) else {
            return f64::NAN;
        };
        CoreWorldSpace::distance(first, second)
    }

    #[func]
    fn rebase_to(&mut self, new_origin: Vector3) -> Vector3 {
        self.rebase_to_components(
            f64::from(new_origin.x),
            f64::from(new_origin.y),
            f64::from(new_origin.z),
        )
    }

    #[func]
    fn rebase_to_components(&mut self, new_x: f64, new_y: f64, new_z: f64) -> Vector3 {
        to_vector3(self.core.rebase_to([new_x, new_y, new_z]))
    }
}

fn to_array(vector: Vector3) -> [f64; 3] {
    [
        f64::from(vector.x),
        f64::from(vector.y),
        f64::from(vector.z),
    ]
}

fn to_components(array: PackedFloat64Array) -> Option<[f64; 3]> {
    (array.len() == 3).then(|| [array[0], array[1], array[2]])
}

fn to_vector3(components: [f64; 3]) -> Vector3 {
    Vector3::new(
        components[0] as f32,
        components[1] as f32,
        components[2] as f32,
    )
}
