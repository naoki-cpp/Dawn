## hud_surface_test.gd
##
## Tests for the HudSurface module that owns live HUD refs and delegates panel
## construction/painting to HudManager.
extends GdUnitTestSuite

const HudSurfaceScript = preload("res://scripts/hud_surface.gd")

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


func test_render_updates_all_hud_panels_from_one_frame() -> void:
	var modules: Array = [{"name": "Afterburner", "is_active_module": true, "is_active": true, "cap_forced_off": false}]
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
	var modules: Array = [
		{"module_id": 1, "slot": "High", "name": "Gun", "is_active_module": true},
		{"module_id": 2, "slot": "Low", "name": "Plate", "is_active_module": false},
	]
	var inventory: Array = [{"module_id": 3, "slot": "Mid", "name": "Afterburner"}]

	_surface.set_player_fitting(modules, inventory)

	assert_int(_surface._module_slots.size()).is_equal(1)
	assert_int((_surface._inventory_panel_refs["fitted_rows"] as Array).size()).is_equal(2)
	assert_int((_surface._inventory_panel_refs["inventory_rows"] as Array).size()).is_equal(1)


func test_inventory_panel_hit_helpers_delegate_to_built_panel() -> void:
	_surface.set_player_fitting([], [{"module_id": 3, "slot": "Mid", "name": "Afterburner"}])
	_surface.toggle_inventory_panel()
	await get_tree().process_frame

	var panel: Panel = _surface._inventory_panel_refs["panel"]
	assert_bool(_surface.inventory_panel_consumes(panel.get_global_rect().get_center())).is_true()

	var rows: Array = _surface._inventory_panel_refs["inventory_rows"] as Array
	var row_panel: Panel = rows[0]["panel"]
	var hit: Dictionary = _surface.inventory_panel_row_at(row_panel.get_global_rect().get_center())
	assert_str(hit.get("action", "") as String).is_equal("fit")
