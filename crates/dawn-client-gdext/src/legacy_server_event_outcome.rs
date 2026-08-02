use godot::prelude::*;

/// Parse-only compatibility for GDScript annotations left by the former
/// two-stage event outcome. The receive path never constructs this class.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct ServerEventOutcome {}

#[godot_api]
impl ServerEventOutcome {
    #[func]
    fn dispatch(&self, _target: Gd<Object>) -> bool {
        false
    }
}
