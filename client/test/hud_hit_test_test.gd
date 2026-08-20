## hud_hit_test_test.gd
##
## Unit tests for hud_hit_test.gd's screen-position hit-testing. Building the
## panels under test still goes through HudManager (the two responsibilities
## split apart, but building is still HudManager's job) -- see
## hud_manager_test.gd for the building/update-logic tests.
extends GdUnitTestSuite

const __source: String = "res://scripts/hud_hit_test.gd"
const InventoryRow = preload("res://scripts/inventory_row.gd")

var _hud: CanvasLayer


func before_test() -> void:
	_hud = auto_free(CanvasLayer.new())
	add_child(_hud)


func test_module_slot_at_finds_the_slot_under_a_screen_position() -> void:
	var module_bar: HBoxContainer = HudManager.build_module_bar(_hud)
	var modules: Array[ModuleRow] = [_module({"name": "Gun", "is_active_module": true})]
	var slots: Array[HudManager.ModuleSlotRefs] = HudManager.rebuild_module_bar(module_bar, modules)
	await get_tree().process_frame  ## let layout/anchors resolve global_rect

	var inside_point: Vector2 = slots[0].panel.get_global_rect().get_center()
	assert_int(HudHitTest.module_slot_at(slots, inside_point)).is_equal(0)
	assert_int(HudHitTest.module_slot_at(slots, Vector2(-9999.0, -9999.0))).is_equal(-1)


## Drag-and-drop's drop-target resolution (main.gd) needs to know which
## column a point falls in even when it's empty space below the last row --
## column_at() checks the *list* container's rect, not individual rows.
func test_column_at_identifies_the_fitted_column() -> void:
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	var refs: HudManager.InventoryPanelRefs = HudManager.build_inventory_panel(hud)
	HudManager.toggle_inventory_panel(refs)
	await get_tree().process_frame

	var result: int = HudHitTest.column_at(refs, refs.fitted_list.get_global_rect().position + Vector2(2, 2))
	assert_int(result).is_equal(StationInventoryInteraction.column_fitted())


func test_column_at_identifies_the_ship_cargo_column() -> void:
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	var refs: HudManager.InventoryPanelRefs = HudManager.build_inventory_panel(hud)
	HudManager.toggle_inventory_panel(refs)
	await get_tree().process_frame

	var result: int = HudHitTest.column_at(refs, refs.inventory_list.get_global_rect().position + Vector2(2, 2))
	assert_int(result).is_equal(StationInventoryInteraction.column_ship_cargo())


func test_column_at_identifies_the_station_column() -> void:
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	var refs: HudManager.InventoryPanelRefs = HudManager.build_inventory_panel(hud)
	HudManager.toggle_inventory_panel(refs)
	await get_tree().process_frame

	var result: int = HudHitTest.column_at(refs, refs.station_list.get_global_rect().position + Vector2(2, 2))
	assert_int(result).is_equal(StationInventoryInteraction.column_station())


func test_column_at_identifies_the_ships_column() -> void:
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	var refs: HudManager.InventoryPanelRefs = HudManager.build_inventory_panel(hud)
	HudManager.toggle_inventory_panel(refs)
	await get_tree().process_frame

	var result: int = HudHitTest.column_at(refs, refs.ships_list.get_global_rect().position + Vector2(2, 2))
	assert_int(result).is_equal(StationInventoryInteraction.column_ships())


func test_column_at_returns_empty_string_for_a_point_outside_every_column() -> void:
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	var refs: HudManager.InventoryPanelRefs = HudManager.build_inventory_panel(hud)
	HudManager.toggle_inventory_panel(refs)
	await get_tree().process_frame

	var result: int = HudHitTest.column_at(refs, Vector2(-9999, -9999))
	assert_int(result).is_equal(StationInventoryInteraction.column_none())


func test_column_at_returns_empty_when_the_panel_is_hidden() -> void:
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	var refs: HudManager.InventoryPanelRefs = HudManager.build_inventory_panel(hud)

	assert_int(HudHitTest.column_at(
		refs, refs.fitted_list.get_global_rect().position
	)).is_equal(StationInventoryInteraction.column_none())


func _module(overrides: Dictionary) -> ModuleRow:
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
	)
