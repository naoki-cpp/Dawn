use dawn_client_core::{ModuleKind, ModuleRow as CoreModuleRow, StatDelta};
use godot::prelude::*;

/// `ModuleKind`'s wire-string name, exactly matching what the server sends
/// (`format!("{:?}", slot.def.kind)` in `player_loadout_projection.rs`) --
/// spelled out explicitly rather than relying on `Debug`'s formatting to
/// stay in sync with that string.
pub(crate) fn kind_str(kind: ModuleKind) -> &'static str {
    match kind {
        ModuleKind::Weapon => "Weapon",
        ModuleKind::ShieldBooster => "ShieldBooster",
        ModuleKind::ArmorRepairer => "ArmorRepairer",
        ModuleKind::Propulsion => "Propulsion",
        ModuleKind::Sensor => "Sensor",
        ModuleKind::Rig => "Rig",
        ModuleKind::Tackle => "Tackle",
        ModuleKind::RemoteShieldBooster => "RemoteShieldBooster",
        ModuleKind::RemoteArmorRepairer => "RemoteArmorRepairer",
        ModuleKind::Unknown => "",
    }
}

/// Reverse of `kind_str` -- parses a wire-string kind name passed into
/// `PlayerLoadout.effective_range_for_activation` back into a [`ModuleKind`].
/// Any unrecognized string
/// maps to `Unknown`, matching the old GDScript's permissive `_range_family()`
/// default.
pub(crate) fn parse_kind(kind: &str) -> ModuleKind {
    match kind {
        "Weapon" => ModuleKind::Weapon,
        "ShieldBooster" => ModuleKind::ShieldBooster,
        "ArmorRepairer" => ModuleKind::ArmorRepairer,
        "Propulsion" => ModuleKind::Propulsion,
        "Sensor" => ModuleKind::Sensor,
        "Rig" => ModuleKind::Rig,
        "Tackle" => ModuleKind::Tackle,
        "RemoteShieldBooster" => ModuleKind::RemoteShieldBooster,
        "RemoteArmorRepairer" => ModuleKind::RemoteArmorRepairer,
        _ => ModuleKind::Unknown,
    }
}

/// GDScript-facing view of one fitted module slot (`dawn_client_core::ModuleRow`).
/// Exported `#[var]` fields are a flattened, display-ready snapshot taken at
/// construction time (`wrap`); `inner` (kept for `equals()`/`clone()`) is the
/// source of truth. Field names and `equals()`/`clone()` mirror the old
/// `module_row.gd` so `hud_surface.gd`'s module-bar diffing needs no changes.
#[derive(GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct ModuleRow {
    #[var]
    slot: GString,
    #[var]
    index: i64,
    #[var]
    module_id: i64,
    #[var]
    name: GString,
    #[var]
    kind: GString,
    /// Custom get/set (rather than a plain `#[var]`) so external writes --
    /// GdUnit4 tests mutate this directly, mirroring the old GDScript's
    /// mutable-field style -- keep `inner` (the `equals()`/`clone()` source
    /// of truth) in sync instead of silently going stale.
    #[var(get = get_is_active, set = set_is_active)]
    is_active: bool,
    #[var]
    is_active_module: bool,
    #[var(get = get_forced_reason, set = set_forced_reason)]
    forced_reason: GString,
    /// Client-local runtime state (capacitor cycling), never read from the
    /// wire. Mutated by `PlayerLoadout::apply_module_activation` and
    /// `PlayerLoadout::simulate_modules_capacitor_ticks`.
    #[var]
    cycle_remaining: i64,

    inner: CoreModuleRow,
}

impl ModuleRow {
    pub(crate) fn wrap(inner: CoreModuleRow) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            slot: (&inner.slot).into(),
            index: inner.index as i64,
            module_id: inner.module_id as i64,
            name: (&inner.name).into(),
            kind: kind_str(inner.kind).into(),
            is_active: inner.is_active,
            is_active_module: inner.is_active_module,
            forced_reason: (&inner.forced_reason).into(),
            cycle_remaining: inner.cycle_remaining as i64,
            inner,
        })
    }

    /// A clone of the domain row this instance wraps, for callers (the
    /// static `PlayerLoadout::simulate_modules_capacitor_ticks`)
    /// that need to run `dawn_client_core` logic across a plain `Vec` rather
    /// than through `Gd` handles one at a time.
    pub(crate) fn inner_clone(&self) -> CoreModuleRow {
        self.inner.clone()
    }

    /// Writes back `cycle_remaining` after an external simulation pass
    /// mutated a cloned copy of `Self::inner_clone` -- keeps the exported
    /// `#[var]` and the internal `inner` copy in sync.
    pub(crate) fn apply_simulated_cycle_remaining(&mut self, value: u32) {
        self.inner.cycle_remaining = value;
        self.cycle_remaining = value as i64;
    }
}

#[godot_api]
impl ModuleRow {
    #[func]
    fn get_is_active(&self) -> bool {
        self.is_active
    }

    #[func]
    fn set_is_active(&mut self, value: bool) {
        self.is_active = value;
        self.inner.is_active = value;
    }

    #[func]
    fn get_forced_reason(&self) -> GString {
        self.forced_reason.clone()
    }

    #[func]
    fn set_forced_reason(&mut self, value: GString) {
        self.inner.forced_reason = value.to_string();
        self.forced_reason = value;
    }

    /// Field-value equality (mirrors the old `ModuleRow.equals()` --
    /// `hud_surface.gd`'s module-bar diffing needs value comparison, not
    /// Godot's default reference-identity `==`).
    #[func]
    fn equals(&self, other: Gd<ModuleRow>) -> bool {
        self.inner == other.bind().inner
    }

    /// An independent copy -- `hud_surface.gd` snapshots the previous
    /// frame's modules to diff against the next one, and a mutated live row
    /// must not silently also change the stored snapshot.
    #[func]
    fn clone(&self) -> Gd<ModuleRow> {
        Self::wrap(self.inner.clone())
    }

    /// Debug-only typed fixture for GdUnit. Production rows are created only
    /// from the typed PlayerLoadout projection.
    #[cfg(debug_assertions)]
    #[allow(clippy::too_many_arguments)]
    #[func]
    fn test_fixture(
        slot: GString,
        index: i64,
        module_id: i64,
        name: GString,
        kind: GString,
        is_active: bool,
        is_active_module: bool,
        cap_cost_per_cycle: f64,
        cycle_time_ticks: i64,
    ) -> Gd<ModuleRow> {
        let index = u32::try_from(index).expect("fixture index must fit u32");
        let module_id = u32::try_from(module_id).expect("fixture module ID must fit u32");
        let cycle_time_ticks =
            u32::try_from(cycle_time_ticks).expect("fixture cycle time must fit u32");
        Self::wrap(CoreModuleRow {
            slot: slot.to_string(),
            index,
            module_id,
            name: name.to_string(),
            kind: parse_kind(&kind.to_string()),
            is_active,
            is_active_module,
            cap_cost_per_cycle,
            cycle_time_ticks,
            stat_delta: StatDelta::ZERO,
            cycle_remaining: 0,
            forced_reason: String::new(),
        })
    }
}
