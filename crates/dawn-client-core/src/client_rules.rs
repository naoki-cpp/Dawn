use dawn_core::MIN_WARP_DISTANCE;

/// Client-visible gameplay rules sourced from the shared Rust domain model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClientRules;

impl ClientRules {
    /// Returns the server-authoritative minimum distance for entering warp.
    #[must_use]
    pub const fn min_warp_distance() -> f64 {
        MIN_WARP_DISTANCE
    }
}

#[cfg(test)]
mod tests {
    use super::ClientRules;
    use dawn_core::MIN_WARP_DISTANCE;

    #[test]
    fn warp_distance_is_the_shared_navigation_rule() {
        assert_eq!(ClientRules::min_warp_distance(), MIN_WARP_DISTANCE);
    }
}
