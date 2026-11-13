use dawn_client_core::{ModuleActivationDecision as CoreDecision, ModuleActivationRejection};
use godot::prelude::*;

use crate::id_boundary::ship_id_to_godot;

/// Typed result of resolving a player module-toggle request.
#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct ModuleActivationDecision {
    inner: CoreDecision,
}

impl ModuleActivationDecision {
    pub(crate) fn from_core(inner: CoreDecision) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self { inner })
    }
}

#[godot_api]
impl ModuleActivationDecision {
    #[func]
    fn action(&self) -> GString {
        match &self.inner {
            CoreDecision::Noop => "noop",
            CoreDecision::Activate { .. } => "activate",
            CoreDecision::Deactivate { .. } => "deactivate",
            CoreDecision::Rejected(_) => "rejected",
        }
        .into()
    }

    #[func]
    fn module_id(&self) -> i64 {
        match &self.inner {
            CoreDecision::Activate { module_id, .. }
            | CoreDecision::Deactivate { module_id, .. } => i64::from(module_id.0),
            CoreDecision::Noop | CoreDecision::Rejected(_) => -1,
        }
    }

    #[func]
    fn slot(&self) -> GString {
        match &self.inner {
            CoreDecision::Activate { slot, .. } | CoreDecision::Deactivate { slot, .. } => {
                slot.as_str().into()
            }
            CoreDecision::Noop | CoreDecision::Rejected(_) => GString::default(),
        }
    }

    #[func]
    fn target_ship_id(&self) -> i64 {
        match &self.inner {
            CoreDecision::Activate {
                target_ship_id: Some(target),
                ..
            } => ship_id_to_godot(*target),
            CoreDecision::Noop
            | CoreDecision::Activate {
                target_ship_id: None,
                ..
            }
            | CoreDecision::Deactivate { .. }
            | CoreDecision::Rejected(_) => -1,
        }
    }

    #[func]
    fn rejection_reason(&self) -> GString {
        match &self.inner {
            CoreDecision::Rejected(ModuleActivationRejection::NoLockedTarget) => "no_locked_target",
            CoreDecision::Rejected(ModuleActivationRejection::TargetUnavailable) => {
                "target_unavailable"
            }
            CoreDecision::Rejected(ModuleActivationRejection::OutOfRange) => "out_of_range",
            _ => "",
        }
        .into()
    }
}
