## hud_surface.gd
##
## Owns the live HUD Control references for main.gd. HudManager still builds
## and paints the individual panels; this module gives main.gd one small
## interface for rendering HUD state and hit-testing HUD controls.
extends RefCounted

var _stats_label: Label = null
var _duel_result_label: Label = null
var _status_panel_refs: Dictionary = {}
var _ship_status_refs: Dictionary = {}
var _target_panel_refs: Dictionary = {}
var _module_bar: HBoxContainer = null
var _module_slots: Array = []
var _inventory_panel_refs: Dictionary = {}


func build(parent: Node, hud: CanvasLayer, stats_label: Label) -> void:
	_stats_label = stats_label
	_duel_result_label = HudManager.build_duel_result_overlay(parent)
	_status_panel_refs = HudManager.build_status_panel(hud)
	_ship_status_refs = HudManager.build_ship_status_panel(hud)
	_target_panel_refs = HudManager.build_target_panel(hud)
	_module_bar = HudManager.build_module_bar(hud)
	_inventory_panel_refs = HudManager.build_inventory_panel(hud)


func render(frame: Dictionary) -> void:
	HudManager.update_status_panel(
		_status_panel_refs,
		frame.get("connected", false) as bool,
		frame.get("ship_type_name", "") as String,
		frame.get("system_name", "Unknown") as String,
		frame.get("speed", "-") as String
	)
	HudManager.update_ship_status_panel(
		_ship_status_refs,
		frame.get("player_ship_id", -1) as int,
		frame.get("shield", -1.0) as float,
		frame.get("max_shield", 500.0) as float,
		frame.get("armor", -1.0) as float,
		frame.get("max_armor", 300.0) as float,
		frame.get("hull", -1.0) as float,
		frame.get("max_hull", 200.0) as float,
		frame.get("cap_current", -1.0) as float,
		frame.get("cap_max", 500.0) as float
	)
	HudManager.update_target_panel(
		_target_panel_refs,
		frame.get("lock_target", -1) as int,
		frame.get("target_known", false) as bool,
		frame.get("target_distance", "—") as String,
		frame.get("target_hp", {}) as Dictionary
	)
	HudManager.update_module_bar(_module_slots, frame.get("modules", []) as Array)
	if _stats_label != null:
		_stats_label.text = frame.get("stats_text", "") as String


func set_player_fitting(modules: Array, inventory: Array) -> void:
	_module_slots = HudManager.rebuild_module_bar(_module_bar, modules)
	HudManager.update_inventory_panel(_inventory_panel_refs, modules, inventory)


func toggle_inventory_panel() -> void:
	HudManager.toggle_inventory_panel(_inventory_panel_refs)


func inventory_panel_consumes(pos: Vector2) -> bool:
	return HudManager.inventory_panel_consumes(_inventory_panel_refs, pos)


func inventory_panel_row_at(pos: Vector2) -> Dictionary:
	return HudManager.inventory_panel_row_at(_inventory_panel_refs, pos)


func module_slot_at(pos: Vector2) -> int:
	return HudManager.module_slot_at(_module_slots, pos)


func show_duel_result(victory: bool) -> void:
	HudManager.show_duel_result(_duel_result_label, victory)


func hide_duel_result() -> void:
	HudManager.hide_duel_result(_duel_result_label)
