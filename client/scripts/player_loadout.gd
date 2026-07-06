## player_loadout.gd
##
## Client-side deep module for the PlayerLoadout wire message. The server emits
## one PlayerLoadout payload, but this module owns the richer client
## concept behind it: fitted modules, ship inventory, station inventory, dock
## context, activation semantics, and capacitor-cycle runtime state.
##
## `_modules`/`_inventory`/`_station_inventory` are typed (ModuleRow/ItemRow,
## see those files) rather than raw Dictionary rows, so a wire-format mismatch
## fails loudly (ModuleRow/ItemRow.from_json logs and drops the row) instead
## of every caller silently defaulting on a missing key.
extends RefCounted

const ModuleRow = preload("res://scripts/module_row.gd")
const ItemRow = preload("res://scripts/item_row.gd")

var _modules: Array[ModuleRow] = []
var _inventory: Array[ItemRow] = []
var _station_inventory: Array[ItemRow] = []
var _tick: int = 0
var _docked_station_id: int = -1
var _docked_station_name: String = ""
var _slot_capacity: Dictionary = {}


func reset() -> void:
	_modules.clear()
	_inventory.clear()
	_station_inventory.clear()
	_tick = 0
	_docked_station_id = -1
	_docked_station_name = ""
	_slot_capacity.clear()


func apply_payload(payload: Dictionary) -> void:
	var raw_docked_station_id: Variant = payload.get("docked_station_id", null)
	_docked_station_id = -1
	if raw_docked_station_id != null:
		_docked_station_id = raw_docked_station_id as int

	var raw_docked_station_name: Variant = payload.get("docked_station_name", null)
	_docked_station_name = ""
	if raw_docked_station_name != null:
		_docked_station_name = raw_docked_station_name as String

	_modules.clear()
	for entry: Variant in payload.get("modules", []) as Array:
		var row: ModuleRow = ModuleRow.from_json(entry as Dictionary)
		if row != null:
			_modules.append(row)

	_inventory.clear()
	for entry: Variant in payload.get("inventory", []) as Array:
		var row: ItemRow = ItemRow.from_json(entry as Dictionary)
		if row != null:
			_inventory.append(row)

	_station_inventory.clear()
	for entry: Variant in payload.get("station_inventory", []) as Array:
		var row: ItemRow = ItemRow.from_json(entry as Dictionary)
		if row != null:
			_station_inventory.append(row)

	_tick = payload.get("tick", 0) as int
	_slot_capacity = payload.get("slot_capacity", {}) as Dictionary


func tick() -> int:
	return _tick


func dock_status() -> Dictionary:
	return {
		"docked_station_id": _docked_station_id,
		"docked_station_name": _docked_station_name,
		"is_docked": _docked_station_id >= 0,
	}


func hud_snapshot() -> Dictionary:
	return {
		"modules": modules(),
		"inventory": inventory(),
		"station_inventory": station_inventory(),
		"dock_status": dock_status(),
	}


func modules() -> Array[ModuleRow]:
	return _modules


func inventory() -> Array[ItemRow]:
	return _inventory


func station_inventory() -> Array[ItemRow]:
	return _station_inventory


func apply_module_activation(module_id: int, active: bool, forced_reason: String = "") -> void:
	for row: ModuleRow in _modules:
		if row.module_id == module_id:
			row.is_active = active
			row.cycle_remaining = 0
			row.forced_reason = forced_reason
			return


func toggle_at(active_index: int) -> Dictionary:
	var active_count: int = 0
	for row: ModuleRow in _modules:
		if not row.is_active_module:
			continue
		if active_count == active_index:
			return {
				"module_id": row.module_id,
				"slot": row.slot,
				"kind": row.kind,
				"is_active": row.is_active,
				"requires_target": _requires_target(row.kind),
				"effective_range": effective_range_for_activation(row.kind, row.module_id),
			}
		active_count += 1
	return {}


func weapon_ranges() -> Dictionary:
	var optimal: float = 0.0
	var falloff: float = 0.0
	for row: ModuleRow in _modules:
		if not row.is_active:
			continue
		if row.kind != "Weapon":
			continue
		optimal += row.stat_delta.get("weapon_range_add", 0.0) as float
		falloff += row.stat_delta.get("falloff_range_add", 0.0) as float
	return {"optimal": optimal, "falloff": falloff}


func effective_range_for_activation(kind: String, module_id: int) -> float:
	var family: String = _range_family(kind)
	if family == "":
		return -1.0

	var total: float = 0.0
	for row: ModuleRow in _modules:
		var is_this_module: bool = row.module_id == module_id
		if not is_this_module and not row.is_active:
			continue
		if _range_family(row.kind) != family:
			continue
		match family:
			"weapon":
				total += (row.stat_delta.get("weapon_range_add", 0.0) as float) \
					+ (row.stat_delta.get("falloff_range_add", 0.0) as float)
			"tackle":
				total += row.stat_delta.get("tackle_range_add", 0.0) as float
			"repair":
				total += row.stat_delta.get("repair_range_add", 0.0) as float
	return total


func simulate_capacitor_ticks(cap_current: float, cap_max: float, cap_recharge: float, ticks: int) -> float:
	return simulate_modules_capacitor_ticks(_modules, cap_current, cap_max, cap_recharge, ticks)


static func simulate_modules_capacitor_ticks(
	module_rows: Array[ModuleRow],
	cap_current: float,
	cap_max: float,
	cap_recharge: float,
	ticks: int,
) -> float:
	var cap: float = cap_current
	for _cap_tick_step: int in range(ticks):
		cap = minf(cap + cap_recharge, cap_max)
		for row: ModuleRow in module_rows:
			if not row.is_active_module:
				continue
			if not row.is_active:
				continue

			if row.cycle_remaining == 0:
				if row.cap_cost_per_cycle <= 0.0 or cap >= row.cap_cost_per_cycle:
					cap -= row.cap_cost_per_cycle
					row.cycle_remaining = row.cycle_time_ticks
			else:
				row.cycle_remaining -= 1
		cap = maxf(cap, 0.0)
	return cap


func _requires_target(kind: String) -> bool:
	return _range_family(kind) != ""


func _range_family(kind: String) -> String:
	match kind:
		"Weapon":
			return "weapon"
		"Tackle":
			return "tackle"
		"RemoteShieldBooster", "RemoteArmorRepairer":
			return "repair"
		_:
			return ""
