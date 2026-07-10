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

/// Reverse of [`kind_str`] -- parses a wire-string kind name (as GDScript
/// passes into `PlayerLoadout.effective_range_for_activation` or a
/// `from_json` payload) back into a [`ModuleKind`]. Any unrecognized string
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

/// Godot `Dictionary` value type used by [`ModuleRow::from_json`] --
/// matches `loadout_gd::Dict`.
type Dict = Dictionary<Variant, Variant>;

const REQUIRED_KEYS: &[&str] = &[
    "slot",
    "index",
    "module_id",
    "name",
    "kind",
    "is_active",
    "is_active_module",
    "cap_cost_per_cycle",
    "cycle_time_ticks",
    "stat_delta",
];

fn stat_delta_f64(stat_delta: &Dict, key: &str) -> f64 {
    stat_delta
        .get(key)
        .and_then(|v| v.try_to::<f64>().ok())
        .unwrap_or(0.0)
}

/// GDScript-facing view of one fitted module slot (`dawn_client_core::ModuleRow`).
/// Exported `#[var]` fields are a flattened, display-ready snapshot taken at
/// [`Self::wrap`] time; `inner` (kept for `equals()`/`clone()`) is the
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
    /// wire. Mutated by [`crate::PlayerLoadout::apply_module_activation`] and
    /// [`crate::PlayerLoadout::simulate_modules_capacitor_ticks`].
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
    /// static [`crate::PlayerLoadout::simulate_modules_capacitor_ticks`])
    /// that need to run `dawn_client_core` logic across a plain `Vec` rather
    /// than through `Gd` handles one at a time.
    pub(crate) fn inner_clone(&self) -> CoreModuleRow {
        self.inner.clone()
    }

    /// Writes back `cycle_remaining` after an external simulation pass
    /// mutated a cloned copy of [`Self::inner_clone`] -- keeps the exported
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

    /// Parses one module row out of a plain (non-wire-JSON) `Dictionary` --
    /// used directly by GdUnit4 tests that build a module list by hand
    /// (`world_session_test.gd`), mirroring the old `module_row.gd::from_json`.
    /// Returns `null` (after an error log) if a required key is missing,
    /// same "fail loudly, drop the row" contract as before.
    #[func]
    fn from_json(src: Dict) -> Variant {
        for key in REQUIRED_KEYS {
            if src.get(*key).is_none() {
                godot_error!("ModuleRow.from_json: invalid module row, missing '{key}'");
                return Variant::nil();
            }
        }

        let get_gstring = |key: &str| -> String {
            src.get(key)
                .and_then(|v| v.try_to::<GString>().ok())
                .map(|s| s.to_string())
                .unwrap_or_default()
        };
        let get_i64 = |key: &str| -> i64 {
            src.get(key)
                .and_then(|v| v.try_to::<i64>().ok())
                .unwrap_or(0)
        };
        let get_f64 = |key: &str| -> f64 {
            src.get(key)
                .and_then(|v| v.try_to::<f64>().ok())
                .unwrap_or(0.0)
        };
        let get_bool = |key: &str| -> bool {
            src.get(key)
                .and_then(|v| v.try_to::<bool>().ok())
                .unwrap_or(false)
        };

        let stat_delta: Dict = src
            .get("stat_delta")
            .and_then(|v| v.try_to::<Dict>().ok())
            .unwrap_or_default();

        let row = CoreModuleRow {
            slot: get_gstring("slot"),
            index: get_i64("index") as u32,
            module_id: get_i64("module_id") as u32,
            name: get_gstring("name"),
            kind: parse_kind(&get_gstring("kind")),
            is_active: get_bool("is_active"),
            is_active_module: get_bool("is_active_module"),
            cap_cost_per_cycle: get_f64("cap_cost_per_cycle"),
            cycle_time_ticks: get_i64("cycle_time_ticks") as u32,
            stat_delta: StatDelta {
                weapon_damage_add: stat_delta_f64(&stat_delta, "weapon_damage_add"),
                weapon_range_add: stat_delta_f64(&stat_delta, "weapon_range_add"),
                falloff_range_add: stat_delta_f64(&stat_delta, "falloff_range_add"),
                tracking_speed_add: stat_delta_f64(&stat_delta, "tracking_speed_add"),
                speed_multiplier: {
                    let v = stat_delta_f64(&stat_delta, "speed_multiplier");
                    if v == 0.0 {
                        1.0
                    } else {
                        v
                    }
                },
                mass_add: stat_delta_f64(&stat_delta, "mass_add"),
                max_shield_add: stat_delta_f64(&stat_delta, "max_shield_add"),
                max_armor_add: stat_delta_f64(&stat_delta, "max_armor_add"),
                max_hull_add: stat_delta_f64(&stat_delta, "max_hull_add"),
                tackle_range_add: stat_delta_f64(&stat_delta, "tackle_range_add"),
                repair_range_add: stat_delta_f64(&stat_delta, "repair_range_add"),
            },
            cycle_remaining: 0,
            forced_reason: String::new(),
        };
        Self::wrap(row).to_variant()
    }
}
