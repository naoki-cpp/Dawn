use dawn_client_core::ClientRules as CoreClientRules;
use godot::prelude::*;

/// Godot adapter for gameplay rules shared by the server and client.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct ClientRules {}

#[godot_api]
impl ClientRules {
    #[func]
    fn min_warp_distance(&self) -> f64 {
        CoreClientRules::min_warp_distance()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_the_shared_warp_distance() {
        let rules = ClientRules {};
        assert_eq!(rules.min_warp_distance(), dawn_core::MIN_WARP_DISTANCE);
    }
}
