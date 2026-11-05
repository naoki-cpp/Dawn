//! Stateless celestial projection, independent of scene nodes and GPU resources.
//! Absolute positions are subtracted in f64 before render-space projection.

/// Facts supplied by the caller for one projection, never cached by this module.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectionView {
    pub observer: [f64; 3],
    pub player_render: [f64; 3],
    /// None preserves physical placement when no camera exists.
    pub camera: Option<ProjectionCamera>,
    /// Obtained from the caller's WorldSpace authority.
    pub render_scale: f64,
}

/// Render-space camera facts; projection does not mutate or retain the camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectionCamera {
    pub render_position: [f64; 3],
    pub far: f64,
}

/// Marker policy; extent and radius are expressed in server metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarkerGeometry {
    Clamped,
    Physical { extent: f64 },
    Sphere { radius: f64 },
}

/// Position and sphere scale to apply at the rendering adapter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkerProjection {
    pub render_position: [f64; 3],
    pub body_scale: f64,
}

/// Render-axis direction and apparent angular radius, in radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunProjection {
    pub render_direction: [f64; 3],
    pub angular_radius: f64,
}

/// Pure projection policy. Invalid or unrepresentable inputs return None.
#[derive(Debug)]
pub struct CelestialProjection;

impl CelestialProjection {
    /// Projects a gate, station, or planet using one supplied observer.
    /// The 30,000 render-unit clamp is presentation-only, never gameplay range.
    pub fn marker(
        view: ProjectionView,
        target: [f64; 3],
        geometry: MarkerGeometry,
    ) -> Option<MarkerProjection> {
        if !finite(view.observer)
            || !finite(view.player_render)
            || !finite(target)
            || !view.render_scale.is_finite()
            || view.render_scale <= 0.0
        {
            return None;
        }
        if let Some(camera) = view.camera {
            if !finite(camera.render_position) || !camera.far.is_finite() || camera.far <= 0.0 {
                return None;
            }
        }
        let delta = sub(target, view.observer);
        let distance = length(delta);
        let offset = [
            delta[0] * view.render_scale,
            delta[1] * view.render_scale,
            -delta[2] * view.render_scale,
        ];
        let physical = add(view.player_render, offset);
        let extent = match geometry {
            MarkerGeometry::Clamped => 0.0,
            MarkerGeometry::Physical { extent } => extent,
            MarkerGeometry::Sphere { radius } => radius,
        };
        if !extent.is_finite() || extent < 0.0 || !distance.is_finite() || !finite(physical) {
            return None;
        }
        let fits = view
            .camera
            .is_none_or(|camera| (distance + extent) * view.render_scale <= camera.far);
        let result = match geometry {
            MarkerGeometry::Sphere { .. } if !fits => {
                let camera = view.camera?;
                let relative = sub(physical, camera.render_position);
                let span = length(relative) + extent * view.render_scale;
                if !finite(relative) || !span.is_finite() {
                    return None;
                }
                let scale = (30_000.0_f64.min(camera.far * 0.5) / span.max(0.001)).min(1.0);
                MarkerProjection {
                    render_position: add(camera.render_position, mul(relative, scale)),
                    body_scale: scale,
                }
            }
            MarkerGeometry::Physical { .. } | MarkerGeometry::Sphere { .. } if fits => {
                MarkerProjection {
                    render_position: physical,
                    body_scale: 1.0,
                }
            }
            _ => MarkerProjection {
                render_position: add(
                    view.player_render,
                    mul(offset, (30_000.0 / length(offset).max(30_000.0)).min(1.0)),
                ),
                body_scale: 1.0,
            },
        };
        (finite(result.render_position) && result.body_scale.is_finite()).then_some(result)
    }

    /// None includes an observer inside the photosphere or within one metre.
    pub fn sun(observer: [f64; 3], position: [f64; 3], radius: f64) -> Option<SunProjection> {
        if !finite(observer) || !finite(position) || !radius.is_finite() || radius < 0.0 {
            return None;
        }
        let delta = sub(position, observer);
        let distance = length(delta);
        if !distance.is_finite() || distance <= radius.max(1.0) {
            return None;
        }
        Some(SunProjection {
            render_direction: [
                delta[0] / distance,
                delta[1] / distance,
                -delta[2] / distance,
            ],
            angular_radius: (radius / distance).clamp(0.0, 1.0).asin(),
        })
    }
}

fn finite(v: [f64; 3]) -> bool {
    v.iter().all(|v| v.is_finite())
}
fn length(v: [f64; 3]) -> f64 {
    v[0].hypot(v[1]).hypot(v[2])
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: [f64; 3], b: f64) -> [f64; 3] {
    [a[0] * b, a[1] * b, a[2] * b]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn view() -> ProjectionView {
        ProjectionView {
            observer: [1.0e12; 3],
            player_render: [100.0, 0.0, 0.0],
            camera: None,
            render_scale: 0.1,
        }
    }

    #[test]
    fn near_offsets_preserve_au_precision_and_render_axes() {
        let p = CelestialProjection::marker(
            view(),
            [1.0e12 + 10.0, 1.0e12 - 20.0, 1.0e12 + 30.0],
            MarkerGeometry::Clamped,
        )
        .unwrap();
        assert_eq!(p.render_position, [101.0, -2.0, -3.0]);
        assert_eq!(p.body_scale, 1.0);
    }

    #[test]
    fn gates_clamp_but_physical_markers_without_a_camera_do_not() {
        let target = [1.0e12 + 1.0e9, 1.0e12, 1.0e12];
        assert!(
            (CelestialProjection::marker(view(), target, MarkerGeometry::Clamped)
                .unwrap()
                .render_position[0]
                - 30_100.0)
                .abs()
                < 1e-8
        );
        assert_eq!(
            CelestialProjection::marker(view(), target, MarkerGeometry::Physical { extent: 100.0 })
                .unwrap()
                .render_position[0],
            100_000_100.0
        );
        assert_eq!(
            CelestialProjection::marker(view(), view().observer, MarkerGeometry::Clamped)
                .unwrap()
                .render_position,
            view().player_render
        );
    }

    #[test]
    fn sphere_proxy_preserves_angular_size_and_returns_to_physical_scale() {
        let mut v = view();
        v.camera = Some(ProjectionCamera {
            render_position: [105.0, 10.0, 20.0],
            far: 5.0e6,
        });
        for distance in [1.0e9, 1.0e8, 1.0e7] {
            let target = add(v.observer, [0.0, 0.0, distance]);
            let p =
                CelestialProjection::marker(v, target, MarkerGeometry::Sphere { radius: 6.4e6 })
                    .unwrap();
            let eye = v.camera.unwrap().render_position;
            let expected = length(sub(add(v.player_render, [0.0, 0.0, -distance * 0.1]), eye));
            let drawn = length(sub(p.render_position, eye));
            assert!((6.4e5 * p.body_scale / drawn - 6.4e5 / expected).abs() < 1e-12);
            assert!(drawn + 6.4e5 * p.body_scale < 5.0e6);
            if distance == 1.0e7 {
                assert_eq!(p.body_scale, 1.0);
            } else {
                assert!(p.body_scale < 1.0);
            }
        }
    }

    #[test]
    fn sun_direction_and_angular_radius_follow_the_observer() {
        let far = CelestialProjection::sun([0.0; 3], [0.0, 0.0, 20_000.0], 1000.0).unwrap();
        let near = CelestialProjection::sun([0.0; 3], [0.0, 0.0, 10_000.0], 1000.0).unwrap();
        assert_eq!(near.render_direction, [0.0, 0.0, -1.0]);
        assert_eq!(near.angular_radius, 0.1_f64.asin());
        assert!(near.angular_radius > far.angular_radius);
        assert_eq!(
            CelestialProjection::sun([0.0; 3], [2000.0, 0.0, 0.0], 1000.0)
                .unwrap()
                .angular_radius,
            0.5_f64.asin()
        );
        assert!(CelestialProjection::sun([0.0; 3], [0.0; 3], 1000.0).is_none());
    }

    #[test]
    fn invalid_projection_inputs_are_rejected() {
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            let mut v = view();
            v.render_scale = bad;
            assert!(CelestialProjection::marker(v, [0.0; 3], MarkerGeometry::Clamped).is_none());
            assert!(CelestialProjection::sun([0.0; 3], [100.0; 3], bad).is_none());
        }
        let mut v = view();
        v.camera = Some(ProjectionCamera {
            render_position: [0.0; 3],
            far: 0.0,
        });
        assert!(CelestialProjection::marker(v, [0.0; 3], MarkerGeometry::Clamped).is_none());
        assert!(
            CelestialProjection::marker(view(), [f64::NAN; 3], MarkerGeometry::Clamped).is_none()
        );
        assert!(CelestialProjection::marker(
            view(),
            [0.0; 3],
            MarkerGeometry::Sphere { radius: -1.0 }
        )
        .is_none());
    }
}
