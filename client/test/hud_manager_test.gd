## hud_manager_test.gd
##
## Unit tests for hud_manager.gd's panel construction and update logic.
## Control node properties (Label.text, ProgressBar.value) can be read/
## written without scene-tree membership, so most of these don't need
## add_child(). The exception is module_slot_at(), which calls
## Control.get_global_rect() -- that needs the panel inside a live tree
## with a flushed layout, so it gets its own add_child() + await.
extends GdUnitTestSuite

const __source: String = "res://scripts/hud_manager.gd"

var _hud: CanvasLayer


func before_test() -> void:
	_hud = auto_free(CanvasLayer.new())
	add_child(_hud)


# -- set_stat_bar / set_mini_bar (percentage math) -----------------------------

func test_set_stat_bar_fills_proportionally_and_formats_the_readout() -> void:
	var entry: Dictionary = HudManager.make_stat_bar("SH", Color.WHITE)
	auto_free(entry["row"])  ## make_stat_bar() doesn't parent its row; free it directly
	HudManager.set_stat_bar(entry, 50.0, 200.0)
	assert_float((entry["bar"] as ProgressBar).value).is_equal_approx(25.0, 0.0001)
	assert_str((entry["value"] as Label).text).is_equal("50 / 200")


func test_set_stat_bar_clamps_to_100_percent_when_cur_exceeds_max() -> void:
	var entry: Dictionary = HudManager.make_stat_bar("SH", Color.WHITE)
	auto_free(entry["row"])
	HudManager.set_stat_bar(entry, 999.0, 200.0)
	assert_float((entry["bar"] as ProgressBar).value).is_equal_approx(100.0, 0.0001)


func test_set_stat_bar_treats_zero_max_as_zero_percent() -> void:
	var entry: Dictionary = HudManager.make_stat_bar("SH", Color.WHITE)
	auto_free(entry["row"])
	HudManager.set_stat_bar(entry, 10.0, 0.0)
	assert_float((entry["bar"] as ProgressBar).value).is_equal_approx(0.0, 0.0001)


func test_set_mini_bar_fills_proportionally() -> void:
	var bar: ProgressBar = auto_free(HudManager.make_mini_bar(Color.WHITE))
	HudManager.set_mini_bar(bar, 30.0, 120.0)
	assert_float(bar.value).is_equal_approx(25.0, 0.0001)


# -- update_status_panel -------------------------------------------------------

func test_update_status_panel_shows_online_when_connected() -> void:
	var refs: Dictionary = HudManager.build_status_panel(_hud)
	HudManager.update_status_panel(refs, true, "Magpie", "Alpha", "120 m/s")
	assert_str((refs["conn_label"] as Label).text).is_equal("ONLINE")
	assert_str((refs["name_label"] as Label).text).is_equal("Magpie")
	assert_str((refs["info_label"] as Label).text).is_equal("System Alpha · 120 m/s")


func test_update_status_panel_shows_connecting_when_disconnected() -> void:
	var refs: Dictionary = HudManager.build_status_panel(_hud)
	HudManager.update_status_panel(refs, false, "", "Alpha", "-")
	assert_str((refs["conn_label"] as Label).text).is_equal("CONNECTING...")
	assert_str((refs["name_label"] as Label).text).is_equal("—")


# -- update_ship_status_panel ---------------------------------------------------

func test_update_ship_status_panel_shows_destroyed_when_no_player_ship() -> void:
	var refs: Dictionary = HudManager.build_ship_status_panel(_hud)
	HudManager.update_ship_status_panel(refs, -1, 0.0, 500.0, 0.0, 300.0, 0.0, 200.0, -1.0, 500.0)
	assert_str((refs["bar_hull"]["value"] as Label).text).is_equal("DESTROYED")
	assert_float((refs["bar_shield"]["bar"] as ProgressBar).value).is_equal_approx(0.0, 0.0001)


func test_update_ship_status_panel_assumes_full_when_state_not_yet_received() -> void:
	var refs: Dictionary = HudManager.build_ship_status_panel(_hud)
	HudManager.update_ship_status_panel(refs, 1, -1.0, 500.0, -1.0, 300.0, -1.0, 200.0, -1.0, 500.0)
	assert_float((refs["bar_shield"]["bar"] as ProgressBar).value).is_equal_approx(100.0, 0.0001)


func test_update_ship_status_panel_shows_live_values() -> void:
	var refs: Dictionary = HudManager.build_ship_status_panel(_hud)
	HudManager.update_ship_status_panel(refs, 1, 250.0, 500.0, 600.0, 600.0, 50.0, 200.0, 80.0, 100.0)
	assert_float((refs["bar_shield"]["bar"] as ProgressBar).value).is_equal_approx(50.0, 0.0001)
	assert_float((refs["bar_cap"]["bar"] as ProgressBar).value).is_equal_approx(80.0, 0.0001)


func test_update_ship_status_panel_shows_dash_when_cap_not_yet_received() -> void:
	var refs: Dictionary = HudManager.build_ship_status_panel(_hud)
	HudManager.update_ship_status_panel(refs, 1, 250.0, 500.0, 600.0, 600.0, 50.0, 200.0, -1.0, 100.0)
	assert_str((refs["bar_cap"]["value"] as Label).text).is_equal("-")


# -- update_target_panel ------------------------------------------------------------

func test_update_target_panel_hides_when_no_lock_target() -> void:
	var refs: Dictionary = HudManager.build_target_panel(_hud)
	HudManager.update_target_panel(refs, -1, false, "—", {})
	assert_bool((refs["panel"] as Panel).visible).is_false()


func test_update_target_panel_shows_signal_lost_when_target_left_the_area() -> void:
	var refs: Dictionary = HudManager.build_target_panel(_hud)
	HudManager.update_target_panel(refs, 7, false, "—", {})
	assert_bool((refs["panel"] as Panel).visible).is_true()
	assert_str((refs["dist_label"] as Label).text).is_equal("SIGNAL LOST")


func test_update_target_panel_shows_distance_and_hp_when_target_is_known() -> void:
	var refs: Dictionary = HudManager.build_target_panel(_hud)
	var hp: Dictionary = {"shield": 50.0, "max_shield": 200.0, "armor": 600.0, "max_armor": 600.0, "hull": 200.0, "max_hull": 200.0}
	HudManager.update_target_panel(refs, 7, true, "3.2 km", hp)
	assert_str((refs["dist_label"] as Label).text).is_equal("3.2 km")
	assert_float((refs["bar_shield"] as ProgressBar).value).is_equal_approx(25.0, 0.0001)


func test_update_target_panel_leaves_bars_unchanged_when_no_hp_record_yet() -> void:
	var refs: Dictionary = HudManager.build_target_panel(_hud)
	HudManager.update_target_panel(refs, 7, true, "1.0 km", {"shield": 80.0, "max_shield": 100.0, "armor": 100.0, "max_armor": 100.0, "hull": 100.0, "max_hull": 100.0})
	var before: float = (refs["bar_shield"] as ProgressBar).value
	HudManager.update_target_panel(refs, 7, true, "1.0 km", {})  ## no HP data this frame
	assert_float((refs["bar_shield"] as ProgressBar).value).is_equal_approx(before, 0.0001)


# -- module bar -----------------------------------------------------------------

func test_rebuild_module_bar_skips_passive_modules() -> void:
	var module_bar: HBoxContainer = HudManager.build_module_bar(_hud)
	var modules: Array = [
		{"name": "Small Railgun I", "is_active_module": true},
		{"name": "Basic Shield Extender", "is_active_module": false},
		{"name": "1MN Afterburner", "is_active_module": true},
	]
	var slots: Array = HudManager.rebuild_module_bar(module_bar, modules)
	assert_int(slots.size()).is_equal(2)
	assert_int(slots[0]["module_index"] as int).is_equal(0)
	assert_int(slots[1]["module_index"] as int).is_equal(2)


func test_update_module_bar_marks_cap_forced_off_modules() -> void:
	var module_bar: HBoxContainer = HudManager.build_module_bar(_hud)
	var modules: Array = [{"name": "Gun", "is_active_module": true, "is_active": true, "cap_forced_off": true}]
	var slots: Array = HudManager.rebuild_module_bar(module_bar, modules)
	HudManager.update_module_bar(slots, modules)
	assert_str((slots[0]["state"] as Label).text).is_equal("CAP!")


func test_update_module_bar_marks_active_modules_on() -> void:
	var module_bar: HBoxContainer = HudManager.build_module_bar(_hud)
	var modules: Array = [{"name": "Gun", "is_active_module": true, "is_active": true, "cap_forced_off": false}]
	var slots: Array = HudManager.rebuild_module_bar(module_bar, modules)
	HudManager.update_module_bar(slots, modules)
	assert_str((slots[0]["state"] as Label).text).is_equal("ON")


func test_module_slot_at_finds_the_slot_under_a_screen_position() -> void:
	var module_bar: HBoxContainer = HudManager.build_module_bar(_hud)
	var modules: Array = [{"name": "Gun", "is_active_module": true}]
	var slots: Array = HudManager.rebuild_module_bar(module_bar, modules)
	await get_tree().process_frame  ## let layout/anchors resolve global_rect

	var panel: Panel = slots[0]["panel"]
	var inside_point: Vector2 = panel.get_global_rect().get_center()
	assert_int(HudManager.module_slot_at(slots, inside_point)).is_equal(0)
	assert_int(HudManager.module_slot_at(slots, Vector2(-9999.0, -9999.0))).is_equal(-1)


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
