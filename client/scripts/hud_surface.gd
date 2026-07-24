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

## Last frame's per-panel sub-Dictionary/Array, used by render() to skip
## HudManager calls for panels that haven't changed since the previous
## frame. GDScript's Dictionary/Array `!=` is a deep (value) comparison, so
## this also catches nested values like target_hp.
## _prev_modules is the one exception: its elements are ModuleRow (an
## Object), and Object's `!=` is reference identity, not value comparison --
## see _modules_changed() below.
## All start empty so the very first render() always paints every panel.
var _prev_status: Dictionary = {}
var _prev_ship_status: Dictionary = {}
var _prev_target: Dictionary = {}
var _prev_modules: Array = []
## A loadout can arrive before main.gd has finished building the HUD (notably
## during headless tests). Keep the latest snapshot until the panel refs exist
## so the first paint is not lost and no UI method touches null refs.
var _pending_fitting: Dictionary = {}


func build(parent: Node, hud: CanvasLayer, stats_label: Label) -> void:
	_stats_label = stats_label
	_hud = hud
	_duel_result_label = HudManager.build_duel_result_overlay(parent)
	_status_panel_refs = HudManager.build_status_panel(hud)
	_ship_status_refs = HudManager.build_ship_status_panel(hud)
	_target_panel_refs = HudManager.build_target_panel(hud)
	_module_bar = HudManager.build_module_bar(hud)
	_inventory_panel_refs = HudManager.build_inventory_panel(hud)
	if not _pending_fitting.is_empty():
		var pending := _pending_fitting
		_pending_fitting = {}
		set_player_fitting(
			pending["modules"] as Array,
			pending["inventory"] as Array,
			pending["station_inventory"] as Array,
			pending["owned_ships"] as Array,
			pending["buildable_ship_types"] as Array)


## True when `next` differs from `prev` (deep value comparison). Pulled out
## as its own predicate so the skip decision is unit-testable independent of
## the real Control nodes render() paints.
func _panel_changed(prev: Variant, next: Variant) -> bool:
	return prev != next


## Same purpose as _panel_changed(), but for Array[ModuleRow]: ModuleRow is
## an Object, so `!=` between arrays of them compares references, not the
## field values render() actually needs to notice changing.
func _modules_changed(prev: Array, next: Array) -> bool:
	if prev.size() != next.size():
		return true
	for i: int in range(next.size()):
		var p: ModuleRow = prev[i]
		var n: ModuleRow = next[i]
		if not p.equals(n):
			return true
	return false


## Independent per-element copies, not aliases -- see _prev_modules's
## comment: ModuleRow is mutated in place, so a plain Array.duplicate(true)
## would keep the same object references.
func _clone_modules(modules: Array) -> Array:
	var out: Array = []
	for m: ModuleRow in modules:
		out.append(m.clone())
	return out


func render(frame: Dictionary) -> void:
	var status: Dictionary = {
		"connected": frame.get("connected", false) as bool,
		"ship_type_name": frame.get("ship_type_name", "") as String,
		"system_name": frame.get("system_name", "Unknown") as String,
		"speed": frame.get("speed", "-") as String,
	}
	if _panel_changed(_prev_status, status):
		HudManager.update_status_panel(
			_status_panel_refs,
			status["connected"], status["ship_type_name"], status["system_name"], status["speed"]
		)
		_prev_status = status

	var ship_status: Dictionary = {
		"player_ship_id": frame.get("player_ship_id", -1) as int,
		"shield": frame.get("shield", -1.0) as float,
		"max_shield": frame.get("max_shield", 500.0) as float,
		"armor": frame.get("armor", -1.0) as float,
		"max_armor": frame.get("max_armor", 300.0) as float,
		"hull": frame.get("hull", -1.0) as float,
		"max_hull": frame.get("max_hull", 200.0) as float,
		"cap_current": frame.get("cap_current", -1.0) as float,
		"cap_max": frame.get("cap_max", 500.0) as float,
	}
	if _panel_changed(_prev_ship_status, ship_status):
		HudManager.update_ship_status_panel(
			_ship_status_refs,
			ship_status["player_ship_id"],
			ship_status["shield"], ship_status["max_shield"],
			ship_status["armor"], ship_status["max_armor"],
			ship_status["hull"], ship_status["max_hull"],
			ship_status["cap_current"], ship_status["cap_max"]
		)
		_prev_ship_status = ship_status

	var target: Dictionary = {
		"lock_target": frame.get("lock_target", -1) as int,
		"target_known": frame.get("target_known", false) as bool,
		"target_distance": frame.get("target_distance", "—") as String,
		"target_hp": frame.get("target_hp", {}) as Dictionary,
	}
	if _panel_changed(_prev_target, target):
		HudManager.update_target_panel(
			_target_panel_refs,
			target["lock_target"], target["target_known"], target["target_distance"], target["target_hp"]
		)
		_prev_target = target

	## `modules` here is the *same* ModuleRow objects as the current
	## PlayerLoadout snapshot (frame["modules"] is assigned by reference, not
	## copied) -- and module activation updates mutate those ModuleRow
	## instances in place inside PlayerLoadout.
	## Storing _prev_modules = modules would alias the live array, so any
	## later in-place mutation would silently also "update" _prev_modules,
	## permanently masking the change from this comparison (e.g. a weapon
	## forced OFF by Range Gate never repainted the module bar, since no
	## PlayerFitting resync -- which replaces the array wholesale -- happens
	## for that event). _clone_modules() takes a real, independent snapshot
	## (ModuleRow is an Object, so Array.duplicate(true) alone would not).
	var modules: Array = frame.get("modules", []) as Array
	if _modules_changed(_prev_modules, modules):
		HudManager.update_module_bar(_module_slots, modules)
		_prev_modules = _clone_modules(modules)

	if _stats_label != null:
		_stats_label.text = frame.get("stats_text", "") as String


## PlayerLoadout arrives both after a structural change (Fit/Unfit) and as a
## state-only resync after every Activate/Deactivate (ADR-0035: the server
## corrects the client's optimistic toggle when it rejects an activation,
## e.g. out of range). Only the former needs rebuild_module_bar() (which
## frees and recreates every slot's Control nodes); rebuilding on every
## Activate/Deactivate caused a visible one-frame stutter each time a module
## was toggled, even when the toggle was rejected. Compare the active-module
## identity list first and skip straight to the cheap per-frame update path
## when nothing structural changed.
func set_player_fitting(
	modules: Array, inventory: Array, station_inventory: Array = [], owned_ships: Array = [],
	buildable_ship_types: Array = []
) -> void:
	if _module_bar == null or _inventory_panel_refs == null:
		_pending_fitting = {
			"modules": modules.duplicate(),
			"inventory": inventory.duplicate(),
			"station_inventory": station_inventory.duplicate(),
			"owned_ships": owned_ships.duplicate(),
			"buildable_ship_types": buildable_ship_types.duplicate(),
		}
		return
	if _active_module_signature(modules) != _active_module_signature(_prev_modules):
		_module_slots = HudManager.rebuild_module_bar(_module_bar, modules)
	## rebuild_module_bar() only builds each slot's F-number/name Controls --
	## it never paints state/colour, so this must always run, rebuilt or not.
	HudManager.update_module_bar(_module_slots, modules)
	## Independent snapshot, not an alias -- see the comment in render().
	_prev_modules = _clone_modules(modules)
	HudManager.update_inventory_panel(
		_inventory_panel_refs, modules, inventory, station_inventory, owned_ships,
		buildable_ship_types)


## Toggles the Build ship-type picker (Phase 9B task 10) and forces the panel
## to redraw with the new expand/collapse state; called by main.gd on a
## ACTION_BUILD_TOGGLE click, which doesn't itself carry a new PlayerLoadout
## snapshot to trigger a rebuild.
func toggle_build_picker(modules: Array, inventory: Array, station_inventory: Array,
		owned_ships: Array, buildable_ship_types: Array) -> void:
	_inventory_panel_refs.build_picker_open = not _inventory_panel_refs.build_picker_open
	HudManager.update_inventory_panel(
		_inventory_panel_refs, modules, inventory, station_inventory, owned_ships, buildable_ship_types)


## Identity of the modules rebuild_module_bar() would render as slots, in
## order -- (module_id, slot) pairs for entries with is_active_module true.
## Order matters: rebuild_module_bar() assigns F-numbers by iteration order.
func _active_module_signature(modules: Array) -> Array:
	var sig: Array = []
	for m: ModuleRow in modules:
		if m.is_active_module:
			sig.append([m.module_id, m.slot])
	return sig


func toggle_inventory_panel() -> void:
	HudManager.toggle_inventory_panel(_inventory_panel_refs)


func inventory_panel_consumes(pos: Vector2) -> bool:
	return HudHitTest.inventory_panel_consumes(_inventory_panel_refs, pos)


func inventory_panel_row_at(pos: Vector2) -> InventoryRow:
	return HudHitTest.inventory_panel_row_at(_inventory_panel_refs, pos)


func inventory_panel_column_at(pos: Vector2) -> String:
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
