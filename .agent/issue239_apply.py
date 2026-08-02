from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def path(name: str) -> Path:
    return ROOT / name


def read(name: str) -> str:
    return path(name).read_text(encoding="utf-8")


def write(name: str, content: str) -> None:
    path(name).write_text(content, encoding="utf-8")


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected 1 exact match, found {count}")
    return text.replace(old, new, 1)


def sub_once(text: str, pattern: str, replacement: str, label: str, flags: int = 0) -> str:
    result, count = re.subn(pattern, replacement, text, count=1, flags=flags)
    if count != 1:
        raise RuntimeError(f"{label}: expected 1 regex match, found {count}")
    return result


# Rust GDExtension registration and JSON adapter removal.
text = read("crates/dawn-client-gdext/src/lib.rs")
text = replace_once(text, "mod json_variant;\n", "", "remove json_variant module")
text = replace_once(
    text,
    "pub use client_command_gd::{ClientCommand, ClientMessageDecoder};",
    "pub use client_command_gd::ClientCommand;",
    "remove ClientMessageDecoder export",
)
write("crates/dawn-client-gdext/src/lib.rs", text)

text = read("crates/dawn-client-gdext/src/client_command_gd.rs")
text = replace_once(
    text,
    '''use crate::{
    item_identity_gd::ItemIdentity,
    json_variant::{externally_tagged_to_dict, json_value_to_variant, Dict},
};
''',
    "use crate::item_identity_gd::ItemIdentity;\n",
    "localize command Dict type",
)
text = replace_once(
    text,
    "use godot::prelude::*;\n",
    "use godot::prelude::*;\n\ntype Dict = Dictionary<Variant, Variant>;\n",
    "add command Dict alias",
)
text = sub_once(
    text,
    r"\n/// Decodes a `ClientMessage` binary frame back into a Dictionary[\s\S]*\Z",
    "\n",
    "delete ClientMessageDecoder",
)
write("crates/dawn-client-gdext/src/client_command_gd.rs", text)

text = read("crates/dawn-client-gdext/src/world_session_gd.rs")
text = replace_once(
    text,
    "use crate::json_variant::Dict;\n\n",
    "type Dict = Dictionary<Variant, Variant>;\n\n",
    "localize WorldSession Dict alias",
)
write("crates/dawn-client-gdext/src/world_session_gd.rs", text)

# Typed debug fixtures replace legacy Dictionary constructors.
text = read("crates/dawn-client-gdext/src/module_row_gd.rs")
text = replace_once(
    text,
    '''/// Reverse of `kind_str` -- parses a wire-string kind name (as GDScript
/// passes into `PlayerLoadout.effective_range_for_activation` or a
/// `from_json` payload) back into a [`ModuleKind`]. Any unrecognized string
''',
    '''/// Reverse of `kind_str` -- parses a wire-string kind name passed into
/// `PlayerLoadout.effective_range_for_activation` back into a [`ModuleKind`].
/// Any unrecognized string
''',
    "update parse_kind documentation",
)
text = sub_once(
    text,
    r"\n/// Godot `Dictionary` value type used by `ModuleRow::from_json`[\s\S]*?(?=\n/// GDScript-facing view)",
    "\n",
    "remove module Dictionary parsing helpers",
)
module_fixture = r'''
    /// Debug-only typed fixture for GdUnit. Production rows are created only
    /// from the typed PlayerLoadout projection.
    #[cfg(debug_assertions)]
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
    ) -> Variant {
        let (Ok(index), Ok(module_id), Ok(cycle_time_ticks)) = (
            u32::try_from(index),
            u32::try_from(module_id),
            u32::try_from(cycle_time_ticks),
        ) else {
            return Variant::nil();
        };
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
        .to_variant()
    }
'''
text = sub_once(
    text,
    r"\n    /// Parses one module row out of a plain \(non-wire-JSON\) `Dictionary`[\s\S]*?\n    \}\n\}\s*\Z",
    "\n" + module_fixture + "}\n",
    "replace ModuleRow.from_json",
)
write("crates/dawn-client-gdext/src/module_row_gd.rs", text)

text = read("crates/dawn-client-gdext/src/owned_ship_row_gd.rs")
text = sub_once(
    text,
    r"\ntype Dict = Dictionary<Variant, Variant>;\n\nconst REQUIRED_KEYS:[\s\S]*?;\n\n",
    "\n",
    "remove owned ship Dictionary schema",
)
text = replace_once(
    text,
    '''    pub(crate) fn wrap(row: CoreOwnedShipRow) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            ship_id: i64::try_from(row.ship_id)
                .expect("PlayerLoadout range validation covers owned ship IDs"),
            ship_type_id: row.ship_type_id.map(i64::from).unwrap_or(-1),
            ship_type_name: row.ship_type_name.as_deref().unwrap_or_default().into(),
            docked_station_id: row.docked_station_id.map(i64::from).unwrap_or(-1),
            is_active: row.is_active,
        })
    }
''',
    '''    pub(crate) fn wrap(row: CoreOwnedShipRow) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            ship_id: i64::try_from(row.ship_id)
                .expect("PlayerLoadout range validation covers owned ship IDs"),
            ship_type_id: row.ship_type_id.map(i64::from).unwrap_or(-1),
            ship_type_name: row.ship_type_name.as_deref().unwrap_or_default().into(),
            docked_station_id: row.docked_station_id.map(i64::from).unwrap_or(-1),
            is_active: row.is_active,
        })
    }

    pub(crate) fn inner_clone(&self) -> CoreOwnedShipRow {
        CoreOwnedShipRow {
            ship_id: u64::try_from(self.ship_id)
                .expect("OwnedShipRow stores a validated ship ID"),
            ship_type_id: u32::try_from(self.ship_type_id).ok(),
            ship_type_name: (!self.ship_type_name.is_empty())
                .then(|| self.ship_type_name.to_string()),
            docked_station_id: u32::try_from(self.docked_station_id).ok(),
            is_active: self.is_active,
        }
    }
''',
    "add owned ship inner clone",
)
owned_fixture = r'''#[godot_api]
impl OwnedShipRow {
    /// Debug-only typed fixture for GdUnit. Negative optional IDs and an empty
    /// type name represent absent values at the Godot boundary.
    #[cfg(debug_assertions)]
    #[func]
    fn test_fixture(
        ship_id: i64,
        ship_type_id: i64,
        ship_type_name: GString,
        docked_station_id: i64,
        is_active: bool,
    ) -> Variant {
        let Ok(ship_id) = u64::try_from(ship_id) else {
            return Variant::nil();
        };
        Self::wrap(CoreOwnedShipRow {
            ship_id,
            ship_type_id: u32::try_from(ship_type_id).ok(),
            ship_type_name: (!ship_type_name.is_empty()).then(|| ship_type_name.to_string()),
            docked_station_id: u32::try_from(docked_station_id).ok(),
            is_active,
        })
        .to_variant()
    }
}
'''
text = sub_once(
    text,
    r"#\[godot_api\]\nimpl OwnedShipRow \{[\s\S]*\Z",
    owned_fixture,
    "replace OwnedShipRow.from_json",
)
write("crates/dawn-client-gdext/src/owned_ship_row_gd.rs", text)

text = read("crates/dawn-client-gdext/src/loadout_gd.rs")
loadout_fixture = r'''    /// Debug-only typed fixture for focused GdUnit tests. Production state is
    /// replaced only from a decoded `PlayerLoadoutWire`.
    #[cfg(debug_assertions)]
    #[func]
    fn test_fixture(
        &mut self,
        tick: i64,
        modules: Array<Gd<ModuleRow>>,
        docked_station_id: i64,
        docked_station_name: GString,
        active_ship_id: i64,
        owned_ships: Array<Gd<OwnedShipRow>>,
    ) -> bool {
        let Ok(tick) = u64::try_from(tick) else {
            return false;
        };
        self.loadout = Some(PlayerLoadoutMsg {
            tick,
            modules: modules
                .iter_shared()
                .map(|row| row.bind().inner_clone())
                .collect(),
            inventory: Vec::new(),
            station_inventory: Vec::new(),
            docked_station_id: u32::try_from(docked_station_id).ok(),
            docked_station_name: (!docked_station_name.is_empty())
                .then(|| docked_station_name.to_string()),
            slot_capacity: dawn_client_core::SlotCapacity {
                high: 0,
                mid: 0,
                low: 0,
                rig: 0,
            },
            active_ship_id: u64::try_from(active_ship_id).ok(),
            owned_ships: owned_ships
                .iter_shared()
                .map(|row| row.bind().inner_clone())
                .collect(),
        });
        true
    }

'''
text = sub_once(
    text,
    r"    /// Test/debug-only convenience: builds state directly from a hand-built[\s\S]*?(?=    #\[func\]\n    fn tick)",
    loadout_fixture,
    "replace PlayerLoadout.apply_payload",
)
write("crates/dawn-client-gdext/src/loadout_gd.rs", text)

text = read("crates/dawn-client-gdext/src/world_space_gd.rs")
text = sub_once(
    text,
    r"    #\[func\]\n    fn to_godot\(&self, server_position: Vector3\) -> Vector3 \{[\s\S]*?    \}\n\n",
    "",
    "remove legacy WorldSpace.to_godot",
)
text = sub_once(
    text,
    r"    #\[func\]\n    fn should_rebase\(&self, player_server_position: Vector3\) -> bool \{[\s\S]*?    \}\n\n",
    "",
    "remove legacy WorldSpace.should_rebase",
)
text = sub_once(
    text,
    r"    #\[func\]\n    fn rebase_to\(&mut self, new_origin: Vector3\) -> Vector3 \{[\s\S]*?    \}\n\n",
    "",
    "remove legacy WorldSpace.rebase_to",
)
write("crates/dawn-client-gdext/src/world_space_gd.rs", text)

text = read("crates/dawn-client-gdext/src/session_record_gd.rs")
text = replace_once(
    text,
    '''    /// Server-space components, the same `PackedFloat64Array` shape
    /// `main.gd`'s `_position_components` already consumes.
''',
    '''    /// Canonical f64 server-space components. This remains a
    /// `PackedFloat64Array` until the final WorldSpace rendering conversion.
''',
    "update GateRecord position docs",
)
write("crates/dawn-client-gdext/src/session_record_gd.rs", text)

# Coordinate adapter deletion in GDScript.
text = read("client/scripts/navigation_marker_renderer.gd")
text = replace_once(
    text,
    'const PositionComponents = preload("res://scripts/position_components.gd")\n\n',
    "",
    "remove marker PositionComponents preload",
)
text = replace_once(
    text,
    "static func spawn_gate_markers(gates_root: Node3D, gates: Array, world_scale: float, to_godot_components: Callable) -> void:",
    "static func spawn_gate_markers(gates_root: Node3D, gates: Array, world_scale: float, world: RefCounted) -> void:",
    "change gate marker seam",
)
text = replace_once(
    text,
    "static func spawn_body_markers(bodies_root: Node3D, bodies: Array, world_scale: float, to_godot_components: Callable) -> void:",
    "static func spawn_body_markers(bodies_root: Node3D, bodies: Array, world_scale: float, world: RefCounted) -> void:",
    "change body marker seam",
)
text = replace_once(
    text,
    "static func spawn_station_markers(bodies_root: Node3D, stations: Array, world_scale: float, to_godot_components: Callable) -> void:",
    "static func spawn_station_markers(bodies_root: Node3D, stations: Array, world_scale: float, world: RefCounted) -> void:",
    "change station marker seam",
)
text = replace_once(
    text,
    "\t\tvar gate_pos := PositionComponents.from_value(g.position)",
    "\t\tvar gate_pos: PackedFloat64Array = g.position",
    "typed gate position",
)
text = replace_once(
    text,
    "\t\tmarker.position = to_godot_components.call(gate_pos) as Vector3",
    "\t\tmarker.position = world.call(\"to_godot_components\", gate_pos[0], gate_pos[1], gate_pos[2]) as Vector3",
    "render gate position",
)
text = replace_once(
    text,
    "\t\tvar b_pos   := PositionComponents.from_value(b.position)",
    "\t\tvar b_pos: PackedFloat64Array = b.position",
    "typed body position",
)
text = replace_once(
    text,
    "\t\tvar godot_pos: Vector3 = to_godot_components.call(b_pos) as Vector3",
    "\t\tvar godot_pos: Vector3 = world.call(\"to_godot_components\", b_pos[0], b_pos[1], b_pos[2]) as Vector3",
    "render body position",
)
text = replace_once(
    text,
    "\t\tvar station_pos := PositionComponents.from_value(station.position)",
    "\t\tvar station_pos: PackedFloat64Array = station.position",
    "typed station position",
)
text = replace_once(
    text,
    "\t\tmarker.position = to_godot_components.call(station_pos) as Vector3",
    "\t\tmarker.position = world.call(\"to_godot_components\", station_pos[0], station_pos[1], station_pos[2]) as Vector3",
    "render station position",
)
write("client/scripts/navigation_marker_renderer.gd", text)

text = read("client/scripts/world_presentation.gd")
text = replace_once(
    text,
    'const PositionComponents = preload("res://scripts/position_components.gd")\n',
    "",
    "remove presentation PositionComponents preload",
)
text = replace_once(
    text,
    '''func respawn_navigation_markers(
	gates: Array,
	bodies: Array,
	stations: Array,
	server_components_to_godot: Callable,
	clear_navigation_selection: Callable
) -> void:
''',
    '''func respawn_navigation_markers(
	gates: Array,
	bodies: Array,
	stations: Array,
	clear_navigation_selection: Callable
) -> void:
''',
    "remove presentation coordinate callback",
)
text = replace_once(
    text,
    '''		NavigationMarkerRendererScript.spawn_gate_markers(
			_gates_root, gates, _render_scale(), server_components_to_godot)
''',
    '''		NavigationMarkerRendererScript.spawn_gate_markers(
			_gates_root, gates, _render_scale(), _world)
''',
    "pass world to gate markers",
)
text = replace_once(
    text,
    '''	NavigationMarkerRendererScript.spawn_body_markers(
		_bodies_root, bodies, _render_scale(), server_components_to_godot)
''',
    '''	NavigationMarkerRendererScript.spawn_body_markers(
		_bodies_root, bodies, _render_scale(), _world)
''',
    "pass world to body markers",
)
text = replace_once(
    text,
    '''	NavigationMarkerRendererScript.spawn_station_markers(
		_bodies_root, stations, _render_scale(), server_components_to_godot)
''',
    '''	NavigationMarkerRendererScript.spawn_station_markers(
		_bodies_root, stations, _render_scale(), _world)
''',
    "pass world to station markers",
)
text = replace_once(
    text,
    '''	var shift: Vector3
	if _world.has_method("rebase_to_components"):
		shift = _world.rebase_to_components(new_origin[0], new_origin[1], new_origin[2])
	else:
		## Keep lightweight presentation tests and alternate world adapters working
		## while the production WorldSpace owns the f64-safe implementation.
		shift = _world.rebase_to(Vector3(new_origin[0], new_origin[1], new_origin[2]))
''',
    '''	var shift: Vector3 = _world.call(
		"rebase_to_components", new_origin[0], new_origin[1], new_origin[2]) as Vector3
''',
    "remove Vector3 rebase fallback",
)
text = replace_once(
    text,
    "\tvar star_pos := PositionComponents.from_value(star.position)",
    "\tvar star_pos: PackedFloat64Array = star.position",
    "typed star position",
)
text = replace_once(
    text,
    "\t\tvar marker_server := PositionComponents.from_value(marker.get_meta(meta_key))",
    "\t\tvar marker_server := marker.get_meta(meta_key) as PackedFloat64Array",
    "typed marker metadata",
)
text = sub_once(
    text,
    r"\nfunc _server_to_godot_pos\(p: Vector3\) -> Vector3:\n\treturn _world\.to_godot\(p\)\n",
    "",
    "remove presentation duplicate conversion",
)
write("client/scripts/world_presentation.gd", text)

text = read("client/scripts/main.gd")
text = replace_once(
    text,
    'const PositionComponents = preload("res://scripts/position_components.gd")\n',
    "",
    "remove main PositionComponents preload",
)
text = sub_once(
    text,
    r"\n## Converts a legacy f32 server-space position[\s\S]*?(?=\n## Tracks whether the player ship)",
    "\n",
    "remove legacy main coordinate helper block",
)
for old, new, label in [
    ("_position_components(gate.position)", "gate.position", "gate typed position"),
    ("_position_components(station.position)", "station.position", "station typed position"),
]:
    if old not in text:
        raise RuntimeError(f"{label}: no match")
    text = text.replace(old, new)
text = replace_once(
    text,
    '''	_presentation.respawn_navigation_markers(
		_gates,
		_bodies,
		_stations,
		_server_components_to_godot,
		Callable(_interaction, "clear_navigation_selection")
	)
''',
    '''	_presentation.respawn_navigation_markers(
		_gates,
		_bodies,
		_stations,
		Callable(_interaction, "clear_navigation_selection")
	)
''',
    "remove marker coordinate callback",
)
text = replace_once(
    text,
    "\t\t_velocity_components_to_vec3(record.velocity),",
    "\t\tVector3(record.velocity[0], record.velocity[1], record.velocity[2]),",
    "render record velocity",
)
text = replace_once(
    text,
    '\t\t"set_velocity", _velocity_components_to_vec3(velocity), tick)',
    '\t\t"set_velocity", Vector3(velocity[0], velocity[1], velocity[2]), tick)',
    "render event velocity",
)
text = replace_once(
    text,
    "\t\t_velocity_components_to_vec3(correction.velocity),",
    "\t\tVector3(correction.velocity[0], correction.velocity[1], correction.velocity[2]),",
    "render correction velocity",
)
write("client/scripts/main.gd", text)

# GdUnit typed fixtures.
def replace_module_helper(filename: str) -> None:
    source = read(filename)
    old = '''func _module(overrides: Dictionary) -> ModuleRow:
	var base: Dictionary = {
		"slot": "High", "index": 0, "module_id": 1, "name": "Test Module", "kind": "Weapon",
		"is_active": false, "is_active_module": true,
		"cap_cost_per_cycle": 0.0, "cycle_time_ticks": 10,
		"stat_delta": {},
	}
	for key: String in overrides:
		base[key] = overrides[key]
	return ModuleRow.from_json(base)
'''
    new = '''func _module(overrides: Dictionary) -> ModuleRow:
	var base: Dictionary = {
		"slot": "High", "index": 0, "module_id": 1, "name": "Test Module", "kind": "Weapon",
		"is_active": false, "is_active_module": true,
		"cap_cost_per_cycle": 0.0, "cycle_time_ticks": 10,
	}
	for key: String in overrides:
		base[key] = overrides[key]
	return ModuleRow.test_fixture(
		base.slot as String,
		base.index as int,
		base.module_id as int,
		base.name as String,
		base.kind as String,
		base.is_active as bool,
		base.is_active_module as bool,
		base.cap_cost_per_cycle as float,
		base.cycle_time_ticks as int,
	) as ModuleRow
'''
    source = replace_once(source, old, new, f"{filename} typed module helper")
    write(filename, source)


replace_module_helper("client/test/hud_hit_test_test.gd")
replace_module_helper("client/test/hud_manager_test.gd")
replace_module_helper("client/test/hud_surface_test.gd")


def replace_owned_helper(filename: str) -> None:
    source = read(filename)
    old = '''func _owned_ship(overrides: Dictionary) -> OwnedShipRow:
	var base: Dictionary = {
		"ship_id": 1,
		"ship_type_id": 7,
		"ship_type_name": "Magpie",
		"docked_station_id": 0,
		"is_active": true,
	}
	for key: String in overrides:
		base[key] = overrides[key]
	return OwnedShipRow.from_json(base)
'''
    new = '''func _owned_ship(overrides: Dictionary) -> OwnedShipRow:
	var base: Dictionary = {
		"ship_id": 1,
		"ship_type_id": 7,
		"ship_type_name": "Magpie",
		"docked_station_id": 0,
		"is_active": true,
	}
	for key: String in overrides:
		base[key] = overrides[key]
	return OwnedShipRow.test_fixture(
		base.ship_id as int,
		base.ship_type_id as int,
		base.ship_type_name as String,
		base.docked_station_id as int,
		base.is_active as bool,
	) as OwnedShipRow
'''
    source = replace_once(source, old, new, f"{filename} typed owned ship helper")
    write(filename, source)


replace_owned_helper("client/test/hud_manager_test.gd")
replace_owned_helper("client/test/hud_surface_test.gd")

text = read("client/test/player_loadout_test.gd")
text = sub_once(
    text,
    r"func test_owned_ships_cross_the_boundary_as_typed_rows\(\) -> void:\n[\s\S]*?(?=\n\nfunc test_dictionary_read_projections)",
    '''func test_owned_ships_cross_the_boundary_as_typed_rows() -> void:
	var loadout := PlayerLoadout.new()
	var owned_ship := OwnedShipRow.test_fixture(9, -1, "", -1, false) as OwnedShipRow
	assert_bool(loadout.test_fixture(
		0, [], 4, "Haven", -1, [owned_ship]
	)).is_true()
	var rows: Array = loadout.owned_ships()
	assert_int(rows.size()).is_equal(1)
	assert_bool(rows[0] is OwnedShipRow).is_true()
	var row: OwnedShipRow = rows[0]
	assert_int(row.ship_id).is_equal(9)
	assert_int(row.ship_type_id).is_equal(-1)
	assert_str(row.ship_type_name).is_equal("")
	assert_int(row.docked_station_id).is_equal(-1)
	assert_bool(row.is_active).is_false()
	assert_int(loadout.docked_station_id()).is_equal(4)
	assert_str(loadout.docked_station_name()).is_equal("Haven")
	assert_bool(loadout.is_docked()).is_true()
''',
    "migrate player loadout typed fixture",
)
write("client/test/player_loadout_test.gd", text)

text = read("client/test/world_session_test.gd")
text = sub_once(
    text,
    r"\n## dawn_core::StatDelta requires every field[\s\S]*?\n\}\n\n",
    "\n",
    "remove world session JSON fixture constant",
)
text = sub_once(
    text,
    r"func test_client_ticks_advance_capacitor_without_server_events\(\) -> void:\n[\s\S]*?(?=\n\nfunc test_destroying_opponent)",
    '''func test_client_ticks_advance_capacitor_without_server_events() -> void:
	_apply_initial_state(11)
	var loadout := PlayerLoadout.new()
	var module := ModuleRow.test_fixture(
		"High", 0, 1, "Gun", "Weapon", true, true, 20.0, 10
	) as ModuleRow
	assert_bool(loadout.test_fixture(0, [module], -1, "", -1, [])).is_true()
	_session.advance_client_ticks(1, loadout)
	var modules: Array = loadout.modules()
	assert_int(_session.current_tick()).is_equal(1)
	assert_float(_session.capacitor_status().current).is_equal_approx(20.0, 0.001)
	assert_int((modules[0] as ModuleRow).cycle_remaining).is_equal(10)
''',
    "migrate world session loadout fixture",
)
write("client/test/world_session_test.gd", text)

text = read("client/test/main_test.gd")
text = sub_once(
    text,
    r"\n## dawn_core::StatDelta \(client-side, ADR-0039\) requires every field[\s\S]*?\n\}\n\n",
    "\n",
    "remove main JSON fixture constant",
)
text = sub_once(
    text,
    r"func _module_fixture\(module_id: int, slot: String, active: bool\) -> Dictionary:\n[\s\S]*?(?=\n\n# -- _server_to_godot_pos)",
    '''func _module_fixture(module_id: int, slot: String, active: bool) -> ModuleRow:
	return ModuleRow.test_fixture(
		slot, 0, module_id, "Test Module", "", active, true, 0.0, 10
	) as ModuleRow


func _set_loadout_modules(modules: Array[ModuleRow]) -> void:
	assert_bool(_main._loadout.test_fixture(
		0, modules, -1, "", -1, []
	)).is_true()
''',
    "migrate main module fixtures",
)
text = sub_once(
    text,
    r"\n# -- _server_to_godot_pos -+\n\nfunc test_server_to_godot_pos_flips_z_and_scales\(\) -> void:\n[\s\S]*?(?=\n\nfunc test_warp_hud_guidance)",
    "\n",
    "remove legacy main conversion test",
)
text = replace_once(
    text,
    '''	_main._loadout.apply_payload(JSON.stringify({
		"tick": 12,
		"active_ship_id": 2,
		"docked_station_id": 0,
		"docked_station_name": "Forge Station",
	}))
''',
    '''	assert_bool(_main._loadout.test_fixture(
		12, [], 0, "Forge Station", 2, []
	)).is_true()
''',
    "migrate docked main loadout fixture",
)
write("client/test/main_test.gd", text)

text = read("client/test/world_presentation_test.gd")
text = replace_once(
    text,
    '''	func rebase_to(_new_origin: Vector3) -> Vector3:
		rebase_calls += 1
		return Vector3(1.0, 2.0, 3.0)
''',
    '''	func rebase_to_components(_x: float, _y: float, _z: float) -> Vector3:
		rebase_calls += 1
		return Vector3(1.0, 2.0, 3.0)
''',
    "migrate fake world rebase method",
)
write("client/test/world_presentation_test.gd", text)

text = read("client/test/planet_visibility_test.gd")
text = replace_once(
    text,
    '''		Callable(_main, "_server_components_to_godot"),
		Callable(_main._interaction, "clear_navigation_selection"),
''',
    '''		Callable(_main._interaction, "clear_navigation_selection"),
''',
    "remove planet visibility coordinate callback",
)
write("client/test/planet_visibility_test.gd", text)

# Move command binary semantics into Rust.
text = read("crates/dawn-wire/src/lib.rs")
rust_tests = r'''

#[cfg(test)]
mod client_message_roundtrip_tests {
    use super::*;
    use dawn_core::{PlayerId, ShipId};

    fn roundtrip(message: &ClientMessage) -> ClientMessage {
        ClientMessage::decode(&message.encode()).expect("postcard ClientMessage round trip")
    }

    #[test]
    fn move_command_preserves_f64_target_components() {
        let message = ClientMessage::Command(ClientCommandWire::MoveCommand {
            target: PosWire {
                x: 10.0,
                y: 0.0,
                z: -5.0,
            },
        });
        assert!(matches!(
            roundtrip(&message),
            ClientMessage::Command(ClientCommandWire::MoveCommand {
                target: PosWire {
                    x: 10.0,
                    y: 0.0,
                    z: -5.0
                }
            })
        ));
    }

    #[test]
    fn module_activation_preserves_optional_target() {
        for target_ship_id in [None, Some(9)] {
            let message = ClientMessage::Command(ClientCommandWire::ActivateModuleCommand {
                module_id: 3,
                slot: "High".to_owned(),
                target_ship_id,
            });
            match roundtrip(&message) {
                ClientMessage::Command(ClientCommandWire::ActivateModuleCommand {
                    module_id,
                    slot,
                    target_ship_id: decoded_target,
                }) => {
                    assert_eq!(module_id, 3);
                    assert_eq!(slot, "High");
                    assert_eq!(decoded_target, target_ship_id);
                }
                _ => panic!("unexpected decoded message"),
            }
        }
    }

    #[test]
    fn navigation_targets_keep_their_wire_variants() {
        let approach = ClientMessage::Command(ClientCommandWire::ApproachCommand {
            target: NavigationTargetWire::Ship(7),
        });
        assert!(matches!(
            roundtrip(&approach),
            ClientMessage::Command(ClientCommandWire::ApproachCommand {
                target: NavigationTargetWire::Ship(7)
            })
        ));

        let warp = ClientMessage::Command(ClientCommandWire::WarpCommand {
            target: WarpTargetWire::Body(5),
        });
        assert!(matches!(
            roundtrip(&warp),
            ClientMessage::Command(ClientCommandWire::WarpCommand {
                target: WarpTargetWire::Body(5)
            })
        ));
    }

    #[test]
    fn market_command_preserves_typed_item_identity() {
        let message = ClientMessage::Market(MarketCommandWire::PlaceMarketOrderCommand {
            ship_id: 42,
            item_id: ItemWire::Module { module_id: 5 },
            side: "Ask".to_owned(),
            price: 100,
            quantity: 3,
        });
        assert!(matches!(
            roundtrip(&message),
            ClientMessage::Market(MarketCommandWire::PlaceMarketOrderCommand {
                ship_id: 42,
                item_id: ItemWire::Module { module_id: 5 },
                side,
                price: 100,
                quantity: 3
            }) if side == "Ask"
        ));
    }

    #[test]
    fn hello_preserves_resume_identity() {
        let message = ClientMessage::Hello(HelloMessage {
            resume: Some(ResumeIdentity {
                player_id: PlayerId(7),
                ship_id: ShipId(dawn_core::EntityId::from_raw(42)),
            }),
        });
        assert!(matches!(
            roundtrip(&message),
            ClientMessage::Hello(HelloMessage {
                resume: Some(ResumeIdentity {
                    player_id: PlayerId(7),
                    ship_id
                })
            }) if ship_id == ShipId(dawn_core::EntityId::from_raw(42))
        ));
    }
}
'''
if "mod client_message_roundtrip_tests" in text:
    raise RuntimeError("wire roundtrip tests already present")
text = text.rstrip() + rust_tests + "\n"
write("crates/dawn-wire/src/lib.rs", text)

# Current architecture/testing documentation describes only the surviving path.
text = read("docs/adr/ADR-0039-dawn-client-core-crate.md")
text = text.replace(
    "`ModuleRow.from_json()`/`ItemRow.from_json()` はサーバーの `PlayerLoadout` wire",
    "`ModuleRow`/`ItemRow` は typed `PlayerLoadout` projection",
)
write("docs/adr/ADR-0039-dawn-client-core-crate.md", text)

text = read("docs/adr/ADR-0040-dawn-client-gdext-binding.md")
text = text.replace(
    "- `PlayerLoadout.apply_payload()` は JSON 文字列を受け取る（`dawn-client-core`",
    "- `PlayerLoadout` は decoded `PlayerLoadoutWire` からのみ状態を置換する（`dawn-client-core`",
)
text = text.replace(
    "- `ModuleRow`/`ItemRow` の `from_json(dict: Dictionary) -> Variant` 静的コンストラクタと",
    "- `ModuleRow`/`ItemRow`/`OwnedShipRow` は typed projection と debug-only typed fixture を持ち、",
)
text += '''

## 2026-08-02: legacy adapter removal (#239)

Typed `WorldSession` migration completion after #238 made the JSON fixture seam
unnecessary. `ClientMessageDecoder`, `json_variant.rs`, row `from_json`
constructors, and `PlayerLoadout.apply_payload` were removed. GdUnit uses typed
debug fixtures or the real binary `ServerMessageDecoder` path. Absolute
positions cross the Rust/Godot boundary as `PackedFloat64Array`; narrowing to
`Vector3` happens only at the rendering seam.
'''
write("docs/adr/ADR-0040-dawn-client-gdext-binding.md", text)

text = read("docs/adr/ADR-0042-wire-postcard-protocol.md")
text += '''

## 2026-08-02: typed client test boundary (#239)

The temporary `ClientMessageDecoder` and `json_variant.rs` compatibility layer
were deleted after the typed `WorldSession` path became authoritative. Client
command postcard semantics are now covered by Rust round-trip tests. GdUnit no
longer reconstructs the former externally tagged JSON/Dictionary shape.
'''
write("docs/adr/ADR-0042-wire-postcard-protocol.md", text)

text = read("docs/architecture/architecture-review/client-completed.md")
text += '''

### 2026-08-02 — legacy client adapter removal (#239)

Removed `ClientMessageDecoder`, `json_variant.rs`, JSON row constructors,
`PlayerLoadout.apply_payload`, `PositionComponents`, and duplicate
Dictionary/Vector3 coordinate helpers. GdUnit fixtures now use typed records or
the real binary decoder; postcard command round trips live in `dawn-wire`
tests. Absolute positions remain f64 components until rendering.
'''
write("docs/architecture/architecture-review/client-completed.md", text)

text = read("docs/architecture/architecture-review/server-completed.md")
text += '''

### 2026-08-02 — client binary test boundary cleanup (#239)

The client-side legacy JSON reconstruction decoder introduced during the
postcard migration was removed. `dawn-wire` now owns client command/message
round-trip tests directly, without reproducing the deprecated Dictionary shape.
'''
write("docs/architecture/architecture-review/server-completed.md", text)

text = read("docs/process/godot-client-testing.md")
text = text.replace(
    "  - e.g. _server_to_godot_pos() / _ray_point_distance() / _spectral_color() /",
    "  - e.g. WorldSpace.to_godot_components() / _ray_point_distance() / _spectral_color() /",
)
text += '''

Typed client fixtures must not recreate wire JSON/Dictionary shapes. Prefer
debug-only typed record factories for focused UI tests and
`ServerMessageDecoder.test_outcome()` when testing the real binary inbound path.
'''
write("docs/process/godot-client-testing.md", text)

# Delete obsolete production/test adapters.
for filename in [
    "crates/dawn-client-gdext/src/json_variant.rs",
    "client/scripts/position_components.gd",
    "client/test/client_command_gd_test.gd",
]:
    target = path(filename)
    if not target.exists():
        raise RuntimeError(f"{filename}: expected file to delete")
    target.unlink()

# Search guard for deleted APIs. Historical prose may mention them only in
# dated removal notes; live code must have no caller or registration.
live_suffixes = {".rs", ".gd"}
for needle in [
    "ClientMessageDecoder",
    "PositionComponents",
    ".from_json(",
    "apply_payload(",
    "_server_to_godot_pos",
    "_vec3_from_dict",
    "_position_components_from_dict",
    "_position_components(",
    "_server_components_to_godot",
    "_velocity_from_dict",
]:
    matches: list[str] = []
    for candidate in ROOT.rglob("*"):
        if (
            candidate.is_file()
            and ".git" not in candidate.parts
            and ".agent" not in candidate.parts
            and candidate.suffix in live_suffixes
        ):
            for line_number, line in enumerate(
                candidate.read_text(encoding="utf-8").splitlines(), 1
            ):
                if needle in line:
                    matches.append(f"{candidate.relative_to(ROOT)}:{line_number}:{line}")
    if matches:
        raise RuntimeError(f"live legacy reference {needle!r} remains:\n" + "\n".join(matches))
