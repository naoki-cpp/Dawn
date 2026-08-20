## hud_surface.gd
##
## Owns the live HUD Control references for main.gd. HudManager still builds
## and paints the individual panels; this module gives main.gd one small
## interface for rendering HUD state and hit-testing HUD controls.
extends RefCounted

## ModuleRow is a GDExtension class (dawn-client-gdext, ADR-0039/ADR-0040) --
## globally registered, no preload needed.
const InventoryRow = preload("res://scripts/inventory_row.gd")

var _stats_label: Label = null
var _duel_result_label: Label = null
var _status_panel_refs: HudManager.StatusPanelRefs = null
var _ship_status_refs: HudManager.ShipStatusPanelRefs = null
var _target_panel_refs: HudManager.TargetPanelRefs = null
var _module_bar: HBoxContainer = null
var _module_slots: Array[HudManager.ModuleSlotRefs] = []
var _inventory_panel_refs: HudManager.InventoryPanelRefs = null
var _hud: CanvasLayer = null

## The core HudReadModel owns panel values and dirty decisions. This module
## keeps only scene references and forwards changed typed snapshots to paint.
class FittingSnapshot extends RefCounted:
	var modules: Array
	var inventory: Array
	var station_inventory: Array
	var owned_ships: Array
	var buildable_ship_types: Array

	func _init(
		modules_: Array, inventory_: Array, station_inventory_: Array,
		owned_ships_: Array, buildable_ship_types_: Array
	) -> void:
		modules = modules_
		inventory = inventory_
		station_inventory = station_inventory_
		owned_ships = owned_ships_
		buildable_ship_types = buildable_ship_types_
## A loadout can arrive before main.gd has finished building the HUD (notably
## during headless tests). Keep the latest snapshot until the panel refs exist
## so the first paint is not lost and no UI method touches null refs.
var _pending_fitting: FittingSnapshot = null


func build(parent: Node, hud: CanvasLayer, stats_label: Label) -> void:
	_stats_label = stats_label
	_hud = hud
	_duel_result_label = HudManager.build_duel_result_overlay(parent)
	_status_panel_refs = HudManager.build_status_panel(hud)
	_ship_status_refs = HudManager.build_ship_status_panel(hud)
	_target_panel_refs = HudManager.build_target_panel(hud)
	_module_bar = HudManager.build_module_bar(hud)
	_inventory_panel_refs = HudManager.build_inventory_panel(hud)
	if _pending_fitting != null:
		var pending := _pending_fitting
		_pending_fitting = null
		set_player_fitting(
			pending.modules, pending.inventory, pending.station_inventory,
			pending.owned_ships, pending.buildable_ship_types)


func paint(frame: HudSnapshot) -> void:
	var changes: HudChangeSet = frame.changes
	if changes.status_changed:
		var status: HudStatusPanel = frame.status
		HudManager.update_status_panel(
			_status_panel_refs, status.connected, status.ship_type_name,
			status.system_name, status.speed_text)
	if changes.ship_status_changed:
		var ship_status: HudShipStatusPanel = frame.ship_status
		HudManager.update_ship_status_panel(
			_ship_status_refs, ship_status.player_ship_id,
			ship_status.shield, ship_status.max_shield,
			ship_status.armor, ship_status.max_armor,
			ship_status.hull, ship_status.max_hull,
			ship_status.cap_current, ship_status.cap_max)
	if changes.target_changed:
		var target: HudTargetPanel = frame.target
		HudManager.update_target_panel(
			_target_panel_refs, target.lock_target_id, target.target_known,
			target.distance_text, target.target_hp as ShipHealth)
	if changes.modules_changed:
		if changes.module_structure_changed:
			_module_slots = HudManager.rebuild_module_bar(_module_bar, frame.modules)
		HudManager.update_module_bar(_module_slots, frame.modules)
	if changes.stats_changed and _stats_label != null:
		_stats_label.text = frame.stats.text

## PlayerLoadout structural and state-only updates both refresh the inventory
## surface here. Module-bar repaint/rebuild decisions come from HudReadModel's
## next typed frame, keeping change policy out of this Control adapter.
func set_player_fitting(
	modules: Array, inventory: Array, station_inventory: Array = [], owned_ships: Array = [],
	buildable_ship_types: Array = []
) -> void:
	if _module_bar == null or _inventory_panel_refs == null:
		_pending_fitting = FittingSnapshot.new(
			modules.duplicate(), inventory.duplicate(), station_inventory.duplicate(),
			owned_ships.duplicate(), buildable_ship_types.duplicate())
		return
	HudManager.update_inventory_panel(
		_inventory_panel_refs, modules, inventory, station_inventory, owned_ships,
		buildable_ship_types)


## Toggles the Build ship-type picker (Phase 9B task 10) and forces the panel
## to redraw with the new expand/collapse state; called by main.gd on a
## build-picker toggle click, which doesn't itself carry a new PlayerLoadout
## snapshot to trigger a rebuild.
func toggle_build_picker(modules: Array, inventory: Array, station_inventory: Array,
		owned_ships: Array, buildable_ship_types: Array) -> void:
	_inventory_panel_refs.build_picker_open = not _inventory_panel_refs.build_picker_open
	HudManager.update_inventory_panel(
		_inventory_panel_refs, modules, inventory, station_inventory, owned_ships, buildable_ship_types)


func toggle_inventory_panel() -> void:
	HudManager.toggle_inventory_panel(_inventory_panel_refs)


func inventory_panel_consumes(pos: Vector2) -> bool:
	return HudHitTest.inventory_panel_consumes(_inventory_panel_refs, pos)


func inventory_panel_row_at(pos: Vector2) -> InventoryRow:
	return HudHitTest.inventory_panel_row_at(_inventory_panel_refs, pos)


func inventory_panel_column_at(pos: Vector2) -> int:
	return HudHitTest.column_at(_inventory_panel_refs, pos)


## A small floating Label that follows the cursor while a row drag is in
## progress -- the only visual feedback the hand-rolled drag gesture gets
## (main.gd's DRAG_THRESHOLD_PX / _drag_row state machine). Caller is
## responsible for freeing it (main.gd's _clear_drag_ghost()).
func create_drag_ghost(text: String) -> Label:
	var ghost := Label.new()
	ghost.text = text
	ghost.mouse_filter = Control.MOUSE_FILTER_IGNORE
	ghost.add_theme_color_override("font_color", Color(1.0, 1.0, 1.0, 0.85))
	ghost.z_index = 100
	_hud.add_child(ghost)
	return ghost


func module_slot_at(pos: Vector2) -> int:
	return HudHitTest.module_slot_at(_module_slots, pos)


func show_duel_result(victory: bool) -> void:
	HudManager.show_duel_result(_duel_result_label, victory)


func hide_duel_result() -> void:
	HudManager.hide_duel_result(_duel_result_label)
