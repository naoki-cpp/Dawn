//! Thin Godot conversion adapter for stateless celestial projection.
use dawn_client_core::{
    CelestialProjection as CoreProjection, MarkerGeometry, ProjectionCamera, ProjectionView,
};
use godot::classes::Camera3D;
use godot::prelude::*;

#[derive(Debug, GodotClass)]
#[class(init, base=RefCounted)]
pub struct CelestialProjection {}

#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct CelestialMarkerProjection {
    #[var]
    position: Vector3,
    #[var]
    body_scale: f64,
}

#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct CelestialSunProjection {
    #[var]
    direction: Vector3,
    #[var]
    angular_radius: f64,
}

#[godot_api]
impl CelestialProjection {
    #[func]
    #[allow(clippy::too_many_arguments)]
    fn marker(
        observer: PackedFloat64Array,
        target: PackedFloat64Array,
        player_render: Vector3,
        camera: Option<Gd<Camera3D>>,
        render_scale: f64,
        physical: bool,
        sphere: bool,
        extent: f64,
    ) -> Option<Gd<CelestialMarkerProjection>> {
        let view = ProjectionView {
            observer: components(observer)?,
            player_render: array(player_render),
            render_scale,
            camera: camera.map(|camera| ProjectionCamera {
                render_position: array(camera.get_global_position()),
                far: camera.get_far() as f64,
            }),
        };
        let geometry = if !physical {
            MarkerGeometry::Clamped
        } else if sphere {
            MarkerGeometry::Sphere { radius: extent }
        } else {
            MarkerGeometry::Physical { extent }
        };
        let result = CoreProjection::marker(view, components(target)?, geometry)?;
        let position = vector(result.render_position);
        position.is_finite().then(|| {
            Gd::from_init_fn(|_| CelestialMarkerProjection {
                position,
                body_scale: result.body_scale,
            })
        })
    }

    #[func]
    fn sun(
        observer: PackedFloat64Array,
        position: PackedFloat64Array,
        radius: f64,
    ) -> Option<Gd<CelestialSunProjection>> {
        let result = CoreProjection::sun(components(observer)?, components(position)?, radius)?;
        Some(Gd::from_init_fn(|_| CelestialSunProjection {
            direction: vector(result.render_direction),
            angular_radius: result.angular_radius,
        }))
    }
}

fn components(v: PackedFloat64Array) -> Option<[f64; 3]> {
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}
fn array(v: Vector3) -> [f64; 3] {
    [v.x as f64, v.y as f64, v.z as f64]
}
fn vector(v: [f64; 3]) -> Vector3 {
    Vector3::new(v[0] as f32, v[1] as f32, v[2] as f32)
}
