## hud_manager_test.gd
##
## Unit tests for hud_manager.gd's panel construction and update logic.
## Control node properties (Label.text, ProgressBar.value) can be read/
## written without scene-tree membership, so most of these don't need
## add_child(). Hit-testing (module_slot_at, column_at, etc., which need
## Control.get_global_rect() on a live tree with flushed layout) is tested
## separately in hud_hit_test_test.gd.
extends GdUnitTestSuite

const __source: String = "res://scripts/hud_manager.gd"
const InventoryRow = preload("res://scripts/inventory_row.gd")

var _hud: CanvasLayer


func before_test() -> void:
	_hud = auto_free(CanvasLayer.new())
	add_child(_hud)


## Minimal but schema-complete module row -- callers override only the keys
## the test cares about, matching ModuleRow's required-key validation.
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


func _owned_ship(overrides: Dictionary) -> OwnedShipRow:
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
	)


# -- set_stat_bar / set_mini_bar (percentage math) -----------------------------

func test_set_stat_bar_fills_proportionally_and_formats_the_readout() -> void:
	var entry: HudManager.StatBarRefs = HudManager.make_stat_bar("SH", Color.WHITE)
	auto_free(entry.row)  ## make_stat_bar() doesn't parent its row; free it directly
	HudManager.set_stat_bar(entry, 50.0, 200.0)
	assert_float(entry.bar.value).is_equal_approx(25.0, 0.0001)
	assert_str(entry.value.text).is_equal("50 / 200")


func test_set_stat_bar_clamps_to_100_percent_when_cur_exceeds_max() -> void:
	var entry: HudManager.StatBarRefs = HudManager.make_stat_bar("SH", Color.WHITE)
	auto_free(entry.row)
	HudManager.set_stat_bar(entry, 999.0, 200.0)
	assert_float(entry.bar.value).is_equal_approx(100.0, 0.0001)


func test_set_stat_bar_treats_zero_max_as_zero_percent() -> void:
	var entry: HudManager.StatBarRefs = HudManager.make_stat_bar("SH", Color.WHITE)
	auto_free(entry.row)
	HudManager.set_stat_bar(entry, 10.0, 0.0)
	assert_float(entry.bar.value).is_equal_approx(0.0, 0.0001)


func test_set_mini_bar_fills_proportionally() -> void:
	var bar: ProgressBar = auto_free(HudManager.make_mini_bar(Color.WHITE))
	HudManager.set_mini_bar(bar, 30.0, 120.0)
	assert_float(bar.value).is_equal_approx(25.0, 0.0001)


# -- update_status_panel -------------------------------------------------------

func test_update_status_panel_shows_online_when_connected() -> void:
	var refs: HudManager.StatusPanelRefs = HudManager.build_status_panel(_hud)
	HudManager.update_status_panel(refs, true, "Magpie", "Alpha", "120 m/s")
	assert_str(refs.conn_label.text).is_equal("ONLINE")
	assert_str(refs.name_label.text).is_equal("Magpie")
	assert_str(refs.info_label.text).is_equal("System Alpha · 120 m/s")


func test_update_status_panel_shows_connecting_when_disconnected() -> void:
	var refs: HudManager.StatusPanelRefs = HudManager.build_status_panel(_hud)
	HudManager.update_status_panel(refs, false, "", "Alpha", "-")
	assert_str(refs.conn_label.text).is_equal("CONNECTING...")
	assert_str(refs.name_label.text).is_equal("—")


# -- update_ship_status_panel ---------------------------------------------------

func test_update_ship_status_panel_shows_destroyed_when_no_player_ship() -> void:
	var refs: HudManager.ShipStatusPanelRefs = HudManager.build_ship_status_panel(_hud)
	HudManager.update_ship_status_panel(refs, -1, 0.0, 500.0, 0.0, 300.0, 0.0, 200.0, -1.0, 500.0)
	assert_str(refs.bar_hull.value.text).is_equal("DESTROYED")
	assert_float(refs.bar_shield.bar.value).is_equal_approx(0.0, 0.0001)


func test_update_ship_status_panel_assumes_full_when_state_not_yet_received() -> void:
	var refs: HudManager.ShipStatusPanelRefs = HudManager.build_ship_status_panel(_hud)
	HudManager.update_ship_status_panel(refs, 1, -1.0, 500.0, -1.0, 300.0, -1.0, 200.0, -1.0, 500.0)
	assert_float(refs.bar_shield.bar.value).is_equal_approx(100.0, 0.0001)


func test_update_ship_status_panel_shows_live_values() -> void:
	var refs: HudManager.ShipStatusPanelRefs = HudManager.build_ship_status_panel(_hud)
	HudManager.update_ship_status_panel(refs, 1, 250.0, 500.0, 600.0, 600.0, 50.0, 200.0, 80.0, 100.0)
	assert_float(refs.bar_shield.bar.value).is_equal_approx(50.0, 0.0001)
	assert_float(refs.bar_cap.bar.value).is_equal_approx(80.0, 0.0001)


func test_update_ship_status_panel_shows_dash_when_cap_not_yet_received() -> void:
	var refs: HudManager.ShipStatusPanelRefs = HudManager.build_ship_status_panel(_hud)
	HudManager.update_ship_status_panel(refs, 1, 250.0, 500.0, 600.0, 600.0, 50.0, 200.0, -1.0, 100.0)
	assert_str(refs.bar_cap.value.text).is_equal("-")


# -- update_target_panel ------------------------------------------------------------

func test_update_target_panel_hides_when_no_lock_target() -> void:
	var refs: HudManager.TargetPanelRefs = HudManager.build_target_panel(_hud)
	HudManager.update_target_panel(refs, -1, false, "—", null)
	assert_bool(refs.panel.visible).is_false()


func test_update_target_panel_shows_signal_lost_when_target_left_the_area() -> void:
	var refs: HudManager.TargetPanelRefs = HudManager.build_target_panel(_hud)
	HudManager.update_target_panel(refs, 7, false, "—", null)
	assert_bool(refs.panel.visible).is_true()
	assert_str(refs.dist_label.text).is_equal("SIGNAL LOST")


func test_update_target_panel_shows_distance_and_hp_when_target_is_known() -> void:
	var refs: HudManager.TargetPanelRefs = HudManager.build_target_panel(_hud)
	var hp: ShipHealth = _health(50.0, 200.0, 600.0, 600.0, 200.0, 200.0)
	HudManager.update_target_panel(refs, 7, true, "3.2 km", hp)
	assert_str(refs.dist_label.text).is_equal("3.2 km")
	assert_float(refs.bar_shield.value).is_equal_approx(25.0, 0.0001)


func test_update_target_panel_leaves_bars_unchanged_when_no_hp_record_yet() -> void:
	var refs: HudManager.TargetPanelRefs = HudManager.build_target_panel(_hud)
	HudManager.update_target_panel(refs, 7, true, "1.0 km", _health(80.0, 100.0, 100.0, 100.0, 100.0, 100.0))
	var before: float = refs.bar_shield.value
	HudManager.update_target_panel(refs, 7, true, "1.0 km", null)  ## no HP data this frame
	assert_float(refs.bar_shield.value).is_equal_approx(before, 0.0001)


# -- module bar -----------------------------------------------------------------

func test_rebuild_module_bar_skips_passive_modules() -> void:
	var module_bar: HBoxContainer = HudManager.build_module_bar(_hud)
	var modules: Array[ModuleRow] = [
		_module({"name": "Small Railgun I", "is_active_module": true}),
		_module({"name": "Basic Shield Extender", "is_active_module": false}),
		_module({"name": "1MN Afterburner", "is_active_module": true}),
	]
	var slots: Array[HudManager.ModuleSlotRefs] = HudManager.rebuild_module_bar(module_bar, modules)
	assert_int(slots.size()).is_equal(2)
	assert_int(slots[0].module_index).is_equal(0)
	assert_int(slots[1].module_index).is_equal(2)


func test_update_module_bar_marks_cap_forced_off_modules() -> void:
	var module_bar: HBoxContainer = HudManager.build_module_bar(_hud)
	var modules: Array[ModuleRow] = [_module({"name": "Gun", "is_active_module": true, "is_active": true})]
	modules[0].forced_reason = "cap"
	var slots: Array[HudManager.ModuleSlotRefs] = HudManager.rebuild_module_bar(module_bar, modules)
	HudManager.update_module_bar(slots, modules)
	assert_str(slots[0].state.text).is_equal("CAP!")


func test_update_module_bar_marks_range_forced_off_modules() -> void:
	var module_bar: HBoxContainer = HudManager.build_module_bar(_hud)
	var modules: Array[ModuleRow] = [_module({"name": "Gun", "is_active_module": true, "is_active": true})]
	modules[0].forced_reason = "range"
	var slots: Array[HudManager.ModuleSlotRefs] = HudManager.rebuild_module_bar(module_bar, modules)
	HudManager.update_module_bar(slots, modules)
	assert_str(slots[0].state.text).is_equal("RANGE!")


func test_update_module_bar_marks_active_modules_on() -> void:
	var module_bar: HBoxContainer = HudManager.build_module_bar(_hud)
	var modules: Array[ModuleRow] = [_module({"name": "Gun", "is_active_module": true, "is_active": true})]
	var slots: Array[HudManager.ModuleSlotRefs] = HudManager.rebuild_module_bar(module_bar, modules)
	HudManager.update_module_bar(slots, modules)
	assert_str(slots[0].state.text).is_equal("ON")


# -- duel result overlay -----------------------------------------------------------

func test_show_duel_result_displays_victory_or_defeat() -> void:
	var parent: Node = auto_free(Node.new())
	add_child(parent)
	var label: Label = HudManager.build_duel_result_overlay(parent)
	HudManager.show_duel_result(label, true)
	assert_str(label.text).is_equal("VICTORY")
	assert_bool(label.visible).is_true()

	HudManager.show_duel_result(label, false)
	assert_str(label.text).is_equal("DEFEAT")


func test_hide_duel_result_hides_the_label() -> void:
	var parent: Node = auto_free(Node.new())
	add_child(parent)
	var label: Label = HudManager.build_duel_result_overlay(parent)
	HudManager.show_duel_result(label, true)
	HudManager.hide_duel_result(label)
	assert_bool(label.visible).is_false()


# -- inventory panel ----------------------------------------------------------------

## Regression: the column headers used to be long, unclipped Labels whose
## minimum width equalled their full unwrapped text. HBoxContainer can't
## shrink a child below its minimum size, so the four columns' combined
## minimum width exceeded the panel's own width, and the last column
## (SHIPS) got pushed outside the panel's rect entirely -- rows rendered
## there were invisible to inventory_panel_consumes() (which only tests the
## outer panel's rect), so a docked player's SHIPS rows became unclickable.
func test_inventory_panel_columns_stay_within_the_panels_bounds() -> void:
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	var refs: HudManager.InventoryPanelRefs = HudManager.build_inventory_panel(hud)
	HudManager.update_inventory_panel(refs, [], [], [], [_owned_ship({})])
	HudManager.toggle_inventory_panel(refs)
	await get_tree().process_frame

	var panel_rect: Rect2 = refs.panel.get_global_rect()
	assert_bool(panel_rect.encloses(refs.ships_list.get_global_rect())).is_true()


func test_station_column_always_shows_disassemble_and_build_toggle_rows() -> void:
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	var refs: HudManager.InventoryPanelRefs = HudManager.build_inventory_panel(hud)
	HudManager.update_inventory_panel(refs, [], [], [], [], [])

	var actions: Array = refs.station_rows.map(func(r: InventoryRow) -> String: return r.action)
	assert_array(actions).contains([InventoryRow.ACTION_DISASSEMBLE, InventoryRow.ACTION_BUILD_TOGGLE])


func test_build_picker_is_collapsed_by_default_and_expands_when_toggled() -> void:
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	var refs: HudManager.InventoryPanelRefs = HudManager.build_inventory_panel(hud)
	var buildable := [_buildable(7, "Magpie")]

	HudManager.update_inventory_panel(refs, [], [], [], [], buildable)
	assert_bool(
		refs.station_rows.any(func(r: InventoryRow) -> bool: return r.action == InventoryRow.ACTION_BUILD_SHIP_TYPE)
	).is_false()

	refs.build_picker_open = true
	HudManager.update_inventory_panel(refs, [], [], [], [], buildable)
	var picker_row: InventoryRow = refs.station_rows.filter(
		func(r: InventoryRow) -> bool: return r.action == InventoryRow.ACTION_BUILD_SHIP_TYPE
	)[0]
	assert_int(picker_row.ship_type_id).is_equal(7)


func test_unfit_all_row_is_hidden_when_no_module_is_fitted() -> void:
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	var refs: HudManager.InventoryPanelRefs = HudManager.build_inventory_panel(hud)
	HudManager.update_inventory_panel(refs, [], [], [], [], [])

	assert_bool(
		refs.fitted_rows.any(func(r: InventoryRow) -> bool: return r.action == InventoryRow.ACTION_UNFIT_ALL)
	).is_false()


func test_unfit_all_row_appears_after_the_fitted_modules_when_any_are_fitted() -> void:
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	var refs: HudManager.InventoryPanelRefs = HudManager.build_inventory_panel(hud)
	var modules := [_module({"module_id": 1, "slot": "High", "name": "Gun"})]

	HudManager.update_inventory_panel(refs, modules, [], [], [], [])

	assert_int(refs.fitted_rows.size()).is_equal(2)
	assert_str(refs.fitted_rows[0].action).is_equal(InventoryRow.ACTION_UNFIT)
	assert_str(refs.fitted_rows[1].action).is_equal(InventoryRow.ACTION_UNFIT_ALL)


## Regression: owned_ships_json (serialization.rs) sends docked_station_id/
## ship_type_name as JSON null (not an absent key) for an away/undocked ship
## or an unregistered ship type. Dictionary.get(key, default) only falls back
## to default when the key is absent, not when it's present with a null
## value -- `as int`/`as String` on that null value crashed with "Invalid
## cast: could not convert value to 'int'".
func test_owned_ship_row_handles_null_docked_station_id_and_ship_type_name() -> void:
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	var refs: HudManager.InventoryPanelRefs = HudManager.build_inventory_panel(hud)
	var owned_ships := [_owned_ship({
		"ship_type_id": null,
		"ship_type_name": null,
		"docked_station_id": null,
		"is_active": false,
	})]

	HudManager.update_inventory_panel(refs, [], [], [], owned_ships, [])

	assert_int(refs.ship_rows.size()).is_equal(1)
	assert_int(refs.ship_rows[0].ship_id).is_equal(1)


## module_slot_at() and column_at() tests moved to hud_hit_test_test.gd
## alongside hud_hit_test.gd (architecture-review/client.md C-9).


## Typed fixture builders (session_record_gd.rs). `update_target_panel` takes
## the same `ShipHealth` `WorldSession.ship_health()` returns, and `null` for
## "no HP record this frame".
func _health(shield: float, max_shield: float, armor: float, max_armor: float, hull: float, max_hull: float) -> ShipHealth:
	var h := ShipHealth.new()
	h.shield = shield
	h.max_shield = max_shield
	h.armor = armor
	h.max_armor = max_armor
	h.hull = hull
	h.max_hull = max_hull
	return h


func _buildable(ship_type_id: int, name: String) -> BuildableShipType:
	var b := BuildableShipType.new()
	b.ship_type_id = ship_type_id
	b.name = name
	return b
