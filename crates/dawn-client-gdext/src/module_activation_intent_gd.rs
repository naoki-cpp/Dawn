use dawn_client_core::ActivationIntent;
use godot::prelude::*;

/// Typed result of resolving a module hotkey or slot click.
///
/// The empty value is represented by the object itself rather than by an
/// empty Dictionary. Callers must check `is_none()` before reading the
/// module-specific accessors.
#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct ModuleActivationIntent {
    inner: Option<ActivationIntent>,
}

impl ModuleActivationIntent {
    pub(crate) fn from_core(intent: Option<ActivationIntent>) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self { inner: intent })
    }
}

#[godot_api]
impl ModuleActivationIntent {
    #[func]
    fn is_none(&self) -> bool {
        self.inner.is_none()
    }

    #[func]
    fn module_id(&self) -> i64 {
        self.inner
            .as_ref()
            .map(|intent| i64::from(intent.module_id))
            .unwrap_or(-1)
    }

    #[func]
    fn slot(&self) -> GString {
        self.inner
            .as_ref()
            .map(|intent| intent.slot.as_str())
            .unwrap_or_default()
            .into()
    }

    #[func]
    fn is_active(&self) -> bool {
        self.inner.as_ref().is_some_and(|intent| intent.is_active)
    }

    #[func]
    fn requires_target(&self) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|intent| intent.requires_target)
    }

    #[func]
    fn has_effective_range(&self) -> bool {
        self.inner
            .as_ref()
            .and_then(|intent| intent.effective_range)
            .is_some()
    }

    #[func]
    fn effective_range(&self) -> f64 {
        self.inner
            .as_ref()
            .and_then(|intent| intent.effective_range)
            .unwrap_or_default()
    }
}
