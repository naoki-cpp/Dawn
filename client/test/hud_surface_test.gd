## hud_surface_test.gd
##
## Tests for the HudSurface module that owns live HUD refs and delegates panel
## construction/painting to HudManager.
extends GdUnitTestSuite

const HudSurfaceScript = preload("res://scripts/hud_surface.gd")
const ModuleRow = preload("res://scripts/module_row.gd")
const ItemRow = preload("res://scripts/item_row.gd")

var _parent: Node
var _hud: CanvasLayer
var _stats_label: Label
var _surface: RefCounted


func before_test() -> void:
	_parent = auto_free(Node.new())
	_hud = auto_free(CanvasLayer.new())
	_stats_label = auto_free(Label.new())
	add_child(_parent)
	add_child(_hud)
	_hud.add_child(_stats_label)
	_surface = HudSurfaceScript.new()
	_surface.build(_parent, _hud, _stats_label)


## Minimal but schema-complete rows -- callers override only the keys the
## test cares about, matching ModuleRow/ItemRow's required-key validation.
func _module(overrides: Dictionary) -> ModuleRow:
	var base: Dictionary = {
		"slot": "High", "index": 0, "module_id": 1, "name": "Test Module", "kind": "Weapon",
		"is_active": false, "is_active_module": true,
		"cap_cost_per_cycle": 0.0, "cycle_time_ticks": 10,
		"stat_delta": {},
	}
	for key: String in overrides:
		base[key] = overrides[key]
	return ModuleRow.from_json(base)


func _item(overrides: Dictionary) -> ItemRow:
	var base: Dictionary = {
		"item_type": "Module", "module_id": 1, "ship_type_id": 0,
		"name": "Test Item", "kind": "", "slot": "", "count": 1,
	}
	for key: String in overrides:
		base[key] = overrides[key]
	return ItemRow.from_json(base)


func test_render_updates_all_hud_panels_from_one_frame() -> void:
	var modules: Array[ModuleRow] = [
		_module({"name": "Afterburner", "is_active_module": true, "is_active": true}),
	]
	_surface.set_player_fitting(modules, [])

	_surface.render({
		"connected": true,
		"ship_type_name": "Magpie",
		"system_name": "Alpha",
		"speed": "120 m/s",
		"player_ship_id": 1,
		"shield": 250.0,
		"max_shield": 500.0,
		"armor": 300.0,
		"max_armor": 300.0,
		"hull": 200.0,
		"max_hull": 200.0,
		"cap_current": 75.0,
		"cap_max": 100.0,
		"lock_target": 7,
		"target_known": true,
		"target_distance": "3.2 km",
		"target_hp": {"shield": 50.0, "max_shield": 100.0, "armor": 100.0, "max_armor": 100.0, "hull": 100.0, "max_hull": 100.0},
		"modules": modules,
		"stats_text": "Ships: 2",
	})

	assert_str((_surface._status_panel_refs["conn_label"] as Label).text).is_equal("ONLINE")
	assert_float((_surface._ship_status_refs["bar_cap"]["bar"] as ProgressBar).value).is_equal_approx(75.0, 0.0001)
	assert_str((_surface._target_panel_refs["dist_label"] as Label).text).is_equal("3.2 km")
	assert_str((_surface._module_slots[0]["state"] as Label).text).is_equal("ON")
	assert_str(_stats_label.text).is_equal("Ships: 2")


func test_set_player_fitting_rebuilds_module_slots_and_inventory_rows() -> void:
	var modules: Array[ModuleRow] = [
		_module({"module_id": 1, "slot": "High", "name": "Gun", "is_active_module": true}),
		_module({"module_id": 2, "slot": "Low", "name": "Plate", "is_active_module": false}),
	]
	var inventory: Array[ItemRow] = [
		_item({"item_type": "Module", "module_id": 3, "slot": "Mid", "name": "Afterburner", "count": 2}),
		_item({"item_type": "ScrapMetal", "name": "Scrap Metal", "count": 4}),
	]

	_surface.set_player_fitting(modules, inventory)

	assert_int(_surface._module_slots.size()).is_equal(1)
	assert_int((_surface._inventory_panel_refs["fitted_rows"] as Array).size()).is_equal(2)
	assert_int((_surface._inventory_panel_refs["inventory_rows"] as Array).size()).is_equal(2)
	var rows: Array = _surface._inventory_panel_refs["inventory_rows"] as Array
	assert_str((((rows[1] as Dictionary)["panel"] as Panel).get_child(0) as Label).text).is_equal("Scrap Metal x4")


func test_render_repaints_after_the_modules_array_is_mutated_in_place() -> void:
	## Regression: main.gd passes the live PlayerLoadout modules into frame["modules"] by
	## reference (not a copy), and _apply_player_module_activation (driven
	## by ModuleActivated/Deactivated events, e.g. Range Gate forcing a
	## weapon off out-of-range) mutates that same array's ModuleRow objects in
	## place via PlayerLoadout.apply_module_activation. If render() stored
	## _prev_modules as an alias of that live array instead of a snapshot,
	## the in-place mutation would silently "update" _prev_modules too,
	## permanently masking the change (the module bar staying ON forever
	## even though the ship truly went inactive server-side).
	var modules: Array[ModuleRow] = [
		_module({"module_id": 1, "slot": "High", "name": "Gun", "is_active_module": true, "is_active": true}),
	]
	var frame: Dictionary = {"modules": modules}
	_surface.set_player_fitting(modules, [])
	_surface.render(frame)
	assert_str((_surface._module_slots[0]["state"] as Label).text).is_equal("ON")

	## Mutate the very same ModuleRow object in place, exactly as
	## PlayerLoadout.apply_module_activation does -- no new object is created.
	(modules[0] as ModuleRow).is_active = false

	_surface.render(frame)
	assert_str((_surface._module_slots[0]["state"] as Label).text).is_equal("OFF")


func test_set_player_fitting_reuses_slots_when_active_module_set_is_unchanged() -> void:
	var modules_off: Array[ModuleRow] = [
		_module({"module_id": 1, "slot": "High", "name": "Gun", "is_active_module": true, "is_active": false}),
	]
	_surface.set_player_fitting(modules_off, [])
	var slots_before: Array = _surface._module_slots

	## Activate/Deactivate resyncs only flip is_active/forced_reason -- the
	## module_id/slot identity list is unchanged, so this must reuse the
	## existing slot Controls (no rebuild) rather than tearing them down.
	var modules_on: Array[ModuleRow] = [
		_module({"module_id": 1, "slot": "High", "name": "Gun", "is_active_module": true, "is_active": true}),
	]
	_surface.set_player_fitting(modules_on, [])

	assert_bool(_surface._module_slots == slots_before).is_true()
	assert_str((_surface._module_slots[0]["state"] as Label).text).is_equal("ON")


func test_set_player_fitting_rebuilds_when_active_module_set_changes() -> void:
	var modules_a: Array[ModuleRow] = [
		_module({"module_id": 1, "slot": "High", "name": "Gun", "is_active_module": true, "is_active": false}),
	]
	_surface.set_player_fitting(modules_a, [])
	var slots_before: Array = _surface._module_slots

	## Fit/Unfit changes which modules are in the active set -- this must
	## rebuild since slot indices/Controls no longer correspond 1:1.
	var modules_b: Array[ModuleRow] = [
		_module({"module_id": 1, "slot": "High", "name": "Gun", "is_active_module": true, "is_active": false}),
		_module({"module_id": 5, "slot": "Mid", "name": "Tackle", "is_active_module": true, "is_active": false}),
	]
	_surface.set_player_fitting(modules_b, [])

	assert_int(_surface._module_slots.size()).is_equal(2)
	assert_bool(_surface._module_slots == slots_before).is_false()


func test_panel_changed_is_false_for_equal_dictionaries() -> void:
	var a: Dictionary = {"shield": 250.0, "armor": 300.0}
	var b: Dictionary = {"shield": 250.0, "armor": 300.0}
	assert_bool(_surface._panel_changed(a, b)).is_false()


func test_panel_changed_is_true_when_a_value_differs() -> void:
	var a: Dictionary = {"shield": 250.0, "armor": 300.0}
	var b: Dictionary = {"shield": 200.0, "armor": 300.0}
	assert_bool(_surface._panel_changed(a, b)).is_true()


func test_panel_changed_is_true_for_nested_dictionary_difference() -> void:
	var a: Dictionary = {"target_hp": {"shield": 50.0, "max_shield": 100.0}}
	var b: Dictionary = {"target_hp": {"shield": 40.0, "max_shield": 100.0}}
	assert_bool(_surface._panel_changed(a, b)).is_true()


func test_panel_changed_is_true_for_array_difference() -> void:
	var a: Array = [{"name": "Afterburner"}]
	var b: Array = [{"name": "Plate"}]
	assert_bool(_surface._panel_changed(a, b)).is_true()


func test_modules_changed_is_false_for_equal_module_rows() -> void:
	var a: Array[ModuleRow] = [_module({"module_id": 1})]
	var b: Array[ModuleRow] = [_module({"module_id": 1})]
	assert_bool(_surface._modules_changed(a, b)).is_false()


func test_modules_changed_is_true_when_a_field_differs() -> void:
	var a: Array[ModuleRow] = [_module({"module_id": 1, "is_active": false})]
	var b: Array[ModuleRow] = [_module({"module_id": 1, "is_active": true})]
	assert_bool(_surface._modules_changed(a, b)).is_true()


func test_modules_changed_is_true_when_size_differs() -> void:
	var a: Array[ModuleRow] = [_module({"module_id": 1})]
	var b: Array[ModuleRow] = [_module({"module_id": 1}), _module({"module_id": 2})]
	assert_bool(_surface._modules_changed(a, b)).is_true()


func test_render_called_twice_with_same_frame_does_not_change_painted_values() -> void:
	var frame: Dictionary = {
		"connected": true,
		"ship_type_name": "Magpie",
		"system_name": "Alpha",
		"speed": "120 m/s",
		"shield": 250.0,
		"cap_current": 75.0,
		"cap_max": 100.0,
	}
	_surface.render(frame)
	_surface.render(frame)

	assert_str((_surface._status_panel_refs["conn_label"] as Label).text).is_equal("ONLINE")
	assert_float((_surface._ship_status_refs["bar_cap"]["bar"] as ProgressBar).value).is_equal_approx(75.0, 0.0001)


func test_inventory_panel_hit_helpers_delegate_to_built_panel() -> void:
	_surface.set_player_fitting([], [_item({"module_id": 3, "slot": "Mid", "name": "Afterburner"})])
	_surface.toggle_inventory_panel()
	await get_tree().process_frame

	var panel: Panel = _surface._inventory_panel_refs["panel"]
	assert_bool(_surface.inventory_panel_consumes(panel.get_global_rect().get_center())).is_true()

	var rows: Array = _surface._inventory_panel_refs["inventory_rows"] as Array
	var row_panel: Panel = rows[0]["panel"]
	var hit: Dictionary = _surface.inventory_panel_row_at(row_panel.get_global_rect().get_center())
	assert_str(hit.get("action", "") as String).is_equal("fit")


func test_station_inventory_packaged_ship_row_is_clickable_to_assemble() -> void:
	_surface.set_player_fitting([], [], [
		_item({"item_type": "PackagedShip", "ship_type_id": 7, "name": "Magpie", "count": 1}),
	])
	_surface.toggle_inventory_panel()
	await get_tree().process_frame

	var rows: Array = _surface._inventory_panel_refs["inventory_rows"] as Array
	var row_panel: Panel = rows[0]["panel"]
	var hit: Dictionary = _surface.inventory_panel_row_at(row_panel.get_global_rect().get_center())
	assert_str(hit.get("action", "") as String).is_equal("assemble")
	assert_int(hit.get("ship_type_id", 0) as int).is_equal(7)
