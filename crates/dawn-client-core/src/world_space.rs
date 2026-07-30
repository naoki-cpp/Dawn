//! Floating-origin coordinate transforms used by the Godot client.

/// Scale applied when converting server metres to client render units.
pub const WORLD_SCALE: f64 = 0.1;

/// Distance from the current origin at which the client should rebase.
pub const REBASE_THRESHOLD: f64 = 1_000_000.0;

/// Maintains the client's floating origin in server-space metres.
///
/// All arithmetic stays in `f64` until the Godot adapter narrows the final
/// render coordinates to `Vector3`. This keeps large absolute positions from
/// losing their nearby offsets during the conversion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldSpace {
    origin: [f64; 3],
}

impl Default for WorldSpace {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldSpace {
    /// Creates a world space whose floating origin is at the server origin.
    pub const fn new() -> Self {
        Self { origin: [0.0; 3] }
    }

    /// Returns the current floating origin in absolute server-space metres.
    pub fn origin(&self) -> [f64; 3] {
        self.origin
    }

    /// Returns the authoritative server-to-render scale used by every transform.
    #[must_use]
    pub const fn render_scale() -> f64 {
        WORLD_SCALE
    }

    /// Converts an absolute server position to origin-relative render units.
    pub fn server_to_render(&self, server_position: [f64; 3]) -> [f64; 3] {
        [
            (server_position[0] - self.origin[0]) * WORLD_SCALE,
            (server_position[1] - self.origin[1]) * WORLD_SCALE,
            -(server_position[2] - self.origin[2]) * WORLD_SCALE,
        ]
    }

    /// Converts origin-relative render units back to an absolute server position.
    pub fn render_to_server(&self, render_position: [f64; 3]) -> [f64; 3] {
        [
            render_position[0] / WORLD_SCALE + self.origin[0],
            render_position[1] / WORLD_SCALE + self.origin[1],
            -render_position[2] / WORLD_SCALE + self.origin[2],
        ]
    }

    /// Converts a server-space direction to render-space axes and scale.
    pub fn server_direction_to_render(&self, server_direction: [f64; 3]) -> [f64; 3] {
        [
            server_direction[0] * WORLD_SCALE,
            server_direction[1] * WORLD_SCALE,
            -server_direction[2] * WORLD_SCALE,
        ]
    }

    /// Converts a render-space direction back to server-space axes and scale.
    pub fn render_direction_to_server(&self, render_direction: [f64; 3]) -> [f64; 3] {
        [
            render_direction[0] / WORLD_SCALE,
            render_direction[1] / WORLD_SCALE,
            -render_direction[2] / WORLD_SCALE,
        ]
    }

    /// Converts a non-negative render-space speed to server-space units.
    pub fn render_speed_to_server(&self, render_speed: f64) -> Option<f64> {
        (render_speed.is_finite() && render_speed >= 0.0).then_some(render_speed / WORLD_SCALE)
    }

    /// Returns whether a player position is far enough from the origin to rebase.
    pub fn should_rebase(&self, player_server_position: [f64; 3]) -> bool {
        let delta = [
            player_server_position[0] - self.origin[0],
            player_server_position[1] - self.origin[1],
            player_server_position[2] - self.origin[2],
        ];
        squared_length(delta) >= REBASE_THRESHOLD * REBASE_THRESHOLD
    }

    /// Returns the Euclidean distance between two absolute server positions.
    pub fn distance(first: [f64; 3], second: [f64; 3]) -> f64 {
        squared_length([
            first[0] - second[0],
            first[1] - second[1],
            first[2] - second[2],
        ])
        .sqrt()
    }

    /// Moves the floating origin and returns the corresponding render-space shift.
    pub fn rebase_to(&mut self, new_origin: [f64; 3]) -> [f64; 3] {
        let shift = [
            (self.origin[0] - new_origin[0]) * WORLD_SCALE,
            (self.origin[1] - new_origin[1]) * WORLD_SCALE,
            -(self.origin[2] - new_origin[2]) * WORLD_SCALE,
        ];
        self.origin = new_origin;
        shift
    }
}

fn squared_length(vector: [f64; 3]) -> f64 {
    vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]
}

#[cfg(test)]
mod tests {
    use super::{WorldSpace, REBASE_THRESHOLD, WORLD_SCALE};

    const AU_M: f64 = 149_597_870_700.0;

    #[test]
    fn subtracts_the_origin_before_render_precision_is_narrowed() {
        let mut world = WorldSpace::new();
        world.rebase_to([5.0 * AU_M, 0.0, 0.0]);

        let render = world.server_to_render([5.0 * AU_M + 10.0, 0.0, 0.0]);
        assert!((render[0] - 1.0).abs() < 1e-9);

        let naive = ((5.0 * AU_M + 10.0) * WORLD_SCALE) as f32 as f64;
        assert!((naive - render[0]).abs() > 1.0);
    }

    #[test]
    fn position_transforms_are_mutual_inverses_at_a_moved_origin() {
        let mut world = WorldSpace::new();
        world.rebase_to([3.0 * AU_M, -1_000_000.0, 2_000_000.0]);

        let server = [3.0 * AU_M + 123.25, -999_876.75, 1_999_456.5];
        let round_trip = world.render_to_server(world.server_to_render(server));

        for (actual, expected) in round_trip.into_iter().zip(server) {
            assert!((actual - expected).abs() < 0.01);
        }
    }

    #[test]
    fn rebasing_preserves_relative_render_position() {
        let mut world = WorldSpace::new();
        let player = [5.0 * AU_M + 20.0, -4.0, 8.0];
        let nearby = [5.0 * AU_M + 35.0, 11.0, -2.0];
        let before = world.server_to_render(nearby);

        let shift = world.rebase_to(player);
        let after = world.server_to_render(nearby);

        for ((actual, expected), shift_component) in after.into_iter().zip(before).zip(shift) {
            assert!((actual - (expected + shift_component)).abs() < 1e-9);
        }
    }

    #[test]
    fn rebase_threshold_uses_server_space_distance() {
        let world = WorldSpace::new();

        assert!(!world.should_rebase([REBASE_THRESHOLD / 2.0, 0.0, 0.0]));
        assert!(world.should_rebase([REBASE_THRESHOLD * 2.0, 0.0, 0.0]));
    }

    #[test]
    fn exposes_the_current_origin_for_new_render_tracks() {
        let mut world = WorldSpace::new();
        let origin = [5.0 * AU_M, -2.0, 3.0];
        world.rebase_to(origin);

        assert_eq!(world.origin(), origin);
    }

    #[test]
    fn distance_preserves_large_absolute_coordinate_offsets() {
        let first = [5.0 * AU_M + 10.0, 0.0, 0.0];
        let second = [5.0 * AU_M + 30.0, 0.0, 0.0];

        assert!((WorldSpace::distance(first, second) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn exposes_the_authoritative_render_scale() {
        assert_eq!(WorldSpace::render_scale(), WORLD_SCALE);
    }

    #[test]
    fn direction_transforms_flip_depth_axis_and_round_trip() {
        let world = WorldSpace::new();
        let server_direction = [2.0, -4.0, 8.0];
        let render_direction = world.server_direction_to_render(server_direction);

        assert_eq!(render_direction, [0.2, -0.4, -0.8]);
        assert_eq!(
            world.render_direction_to_server(render_direction),
            server_direction
        );
    }

    #[test]
    fn converts_render_speed_caps_to_server_units() {
        let world = WorldSpace::new();

        assert_eq!(world.render_speed_to_server(2_000.0), Some(20_000.0));
        assert_eq!(world.render_speed_to_server(-1.0), None);
        assert_eq!(world.render_speed_to_server(f64::NAN), None);
    }
}
