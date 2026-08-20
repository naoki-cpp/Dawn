## hud_surface_test.gd
##
## HudSurface is tested at the typed read-model -> paint boundary. Rust owns
## projection and dirty decisions; this suite checks that the existing Control
## tree receives the right typed values and that hit-test/inventory ownership
## remains intact.
extends GdUnitTestSuite

const HudSurfaceScript = preload("res://scripts/hud_surface.gd")
const InventoryRow = preload("res://scripts/inventory_row.gd")

var _parent: Node
var _hud: CanvasLayer
var _stats_label: Label
var _surface: RefCounted
var _session: WorldSession
var _loadout: PlayerLoadout
var _read_model: HudReadModel


func before_test() -> void:
	_parent = auto_free(Node.new())
	_hud = auto_free(CanvasLayer.new())
	_stats_label = auto_free(Label.new())
	add_child(_parent)
	add_child(_hud)
	_hud.add_child(_stats_label)
	_surface = HudSurfaceScript.new()
	_surface.build(_parent, _hud, _stats_label)
	_session = WorldSession.new()
	_loadout = PlayerLoadout.new()
	_read_model = HudReadModel.new()


func _module(overrides: Dictionary = {}) -> ModuleRow:
	var base: Dictionary = {
		"slot": "High", "index": 0, "module_id": 1, "name": "Test Module", "kind": "Weapon",
		"is_active": false, "is_active_module": true,
		"cap_cost_per_cycle": 0.0, "cycle_time_ticks": 10,
	}
	for key: String in overrides:
		base[key] = overrides[key]
	return ModuleRow.test_fixture(
		base.slot as String, base.index as int, base.module_id as int,
		base.name as String, base.kind as String, base.is_active as bool,
		base.is_active_module as bool, base.cap_cost_per_cycle as float,
		base.cycle_time_ticks as int)


func _item(item_id: ItemIdentity, overrides: Dictionary = {}) -> ItemRow:
	return ItemRow.test_fixture(
		item_id, overrides.get("name", "Test Item") as String,
		overrides.get("kind", "") as String, overrides.get("slot", "") as String,
		overrides.get("count", 1) as int) as ItemRow


func _owned_ship(ship_id: int, active: bool) -> OwnedShipRow:
	return OwnedShipRow.test_fixture(ship_id, 7, "Magpie", 0, active)


func _snapshot(modules: Array[ModuleRow] = [], connected: bool = false, speed: float = 0.0) -> HudSnapshot:
	_loadout.test_fixture(0, modules, -1, "", -1, [])
	var facts := HudSceneFacts.new()
	facts.connected = connected
	facts.has_player_speed = connected
	facts.player_speed_units = speed
	facts.nearby_gate_id = -1
	facts.selected_gate_id = -1
	facts.selected_body_id = -1
	facts.selected_station_id = -1
	facts.selected_target_id = -1
	facts.keep_at_range_km = 10.0
	return _read_model.project(_session, _loadout, facts)


func test_paint_updates_status_ship_module_and_stats_from_typed_snapshot() -> void:
	var modules: Array[ModuleRow] = [_module({"name": "Afterburner", "is_active": true})]
	_surface.paint(_snapshot(modules, true, 120.0))

	assert_str(_surface._status_panel_refs.conn_label.text).is_equal("ONLINE")
	assert_str(_surface._status_panel_refs.info_label.text).is_equal("System Unknown · 120 m/s")
	assert_int(_surface._module_slots.size()).is_equal(1)
	assert_str(_surface._module_slots[0].state.text).is_equal("ON")
	assert_str(_stats_label.text).contains("Ships: 0")


func test_paint_uses_core_dirty_decisions_for_equal_and_changed_snapshots() -> void:
	_surface.paint(_snapshot([_module()], true, 120.0))
	var equal := _snapshot([_module()], true, 120.0)
	assert_bool(equal.changes.status_changed).is_false()
	assert_bool(equal.changes.modules_changed).is_false()
	_surface.paint(equal)

	var changed := _snapshot([_module({"is_active": true})], true, 120.0)
	assert_bool(changed.changes.modules_changed).is_true()
	assert_bool(changed.changes.module_structure_changed).is_false()
	_surface.paint(changed)
	assert_str(_surface._module_slots[0].state.text).is_equal("ON")


func test_module_structure_change_rebuilds_slots_at_paint_boundary() -> void:
	_surface.paint(_snapshot([_module({"module_id": 1})]))
	var slots_before: Array = _surface._module_slots
	_surface.paint(_snapshot([
		_module({"module_id": 1}), _module({"module_id": 2, "index": 1})
	]))
	assert_bool(_surface._module_slots == slots_before).is_false()
	assert_int(_surface._module_slots.size()).is_equal(2)


func test_fitting_before_build_is_applied_after_build_without_a_dictionary_frame() -> void:
	var unbuilt: RefCounted = HudSurfaceScript.new()
	var modules: Array[ModuleRow] = [_module({"module_id": 7, "name": "Railgun"})]
	var inventory: Array[ItemRow] = [_item(ItemIdentity.scrap_metal(), {"name": "Scrap Metal", "count": 3})]
	unbuilt.set_player_fitting(modules, inventory)
	unbuilt.build(_parent, _hud, _stats_label)

	assert_int(unbuilt._inventory_panel_refs.inventory_rows.size()).is_equal(1)
	assert_bool(unbuilt._inventory_panel_refs.inventory_rows[0].policy_row.is_cargo()).is_true()


func test_inventory_panel_hit_helpers_delegate_to_built_panel() -> void:
	_surface.set_player_fitting([], [_item(ItemIdentity.module(3), {"slot": "Mid", "name": "Afterburner"})])
	_surface.toggle_inventory_panel()
	await get_tree().process_frame
	var panel: Panel = _surface._inventory_panel_refs.panel
	assert_bool(_surface.inventory_panel_consumes(panel.get_global_rect().get_center())).is_true()
	var row: InventoryRow = _surface._inventory_panel_refs.inventory_rows[0]
	var hit: InventoryRow = _surface.inventory_panel_row_at((row.panel as Panel).get_global_rect().get_center())
	assert_bool(hit.policy_row.is_cargo()).is_true()


func test_station_inventory_packaged_ship_row_is_clickable_to_assemble() -> void:
	_surface.set_player_fitting([], [], [
		_item(ItemIdentity.packaged_ship(7), {"name": "Magpie", "count": 1})
	])
	_surface.toggle_inventory_panel()
	await get_tree().process_frame
	var row: InventoryRow = _surface._inventory_panel_refs.station_rows[0]
	var hit: InventoryRow = _surface.inventory_panel_row_at((row.panel as Panel).get_global_rect().get_center())
	assert_bool(hit.policy_row.is_station_item()).is_true()


func test_owned_ships_roster_lists_active_and_selectable_ships() -> void:
	_surface.set_player_fitting([], [], [], [_owned_ship(1, true), _owned_ship(2, false)])
	_surface.toggle_inventory_panel()
	await get_tree().process_frame
	var rows: Array[InventoryRow] = _surface._inventory_panel_refs.ship_rows
	assert_int(rows.size()).is_equal(2)
	assert_bool(rows[0].policy_row.is_owned_ship_active()).is_true()
	assert_bool(rows[1].policy_row.is_owned_ship_selectable()).is_true()
