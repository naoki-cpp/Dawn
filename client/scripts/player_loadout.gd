## player_loadout.gd
##
## Client-side deep module for the PlayerLoadout wire message. The server emits
## one PlayerLoadout payload, but this module owns the richer client
## concept behind it: fitted modules, ship inventory, station inventory, dock
## context, activation semantics, and capacitor-cycle runtime state.
extends RefCounted

var _modules: Array = []
var _inventory: Array = []
var _station_inventory: Array = []
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
		var src: Dictionary = entry as Dictionary
		var stat_delta: Dictionary = src.get("stat_delta", {}) as Dictionary
		_modules.append({
			"slot": src.get("slot", "") as String,
			"index": src.get("index", 0) as int,
			"module_id": src.get("module_id", 0) as int,
			"name": src.get("name", "?") as String,
			"kind": src.get("kind", "") as String,
			"is_active": src.get("is_active", false) as bool,
			"is_active_module": src.get("is_active_module", false) as bool,
			"cap_cost_per_cycle": src.get("cap_cost_per_cycle", 0.0) as float,
			"cycle_time_ticks": src.get("cycle_time_ticks", 10) as int,
			"cycle_remaining": 0,
			"forced_reason": "",
			"stat_delta": {
				"weapon_range_add": stat_delta.get("weapon_range_add", 0.0) as float,
				"falloff_range_add": stat_delta.get("falloff_range_add", 0.0) as float,
				"tackle_range_add": stat_delta.get("tackle_range_add", 0.0) as float,
				"repair_range_add": stat_delta.get("repair_range_add", 0.0) as float,
			},
		})

	_inventory.clear()
	for entry: Variant in payload.get("inventory", []) as Array:
		var src: Dictionary = entry as Dictionary
		_inventory.append({
			"item_type": src.get("item_type", "Module") as String,
			"module_id": src.get("module_id", 0) as int,
			"ship_type_id": src.get("ship_type_id", 0) as int,
			"name": src.get("name", "?") as String,
			"kind": src.get("kind", "") as String,
			"slot": src.get("slot", "") as String,
			"count": src.get("count", 1) as int,
		})

	_station_inventory.clear()
	for entry: Variant in payload.get("station_inventory", []) as Array:
		var src: Dictionary = entry as Dictionary
		_station_inventory.append({
			"item_type": src.get("item_type", "Module") as String,
			"module_id": src.get("module_id", 0) as int,
			"ship_type_id": src.get("ship_type_id", 0) as int,
			"name": src.get("name", "?") as String,
			"kind": src.get("kind", "") as String,
			"slot": src.get("slot", "") as String,
			"count": src.get("count", 1) as int,
		})

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


func modules() -> Array:
	return _modules


func inventory() -> Array:
	return _inventory


func station_inventory() -> Array:
	return _station_inventory


func apply_module_activation(module_id: int, active: bool, forced_reason: String = "") -> void:
	for entry: Variant in _modules:
		var module: Dictionary = entry as Dictionary
		if module.get("module_id", 0) as int == module_id:
			module["is_active"] = active
			module["cycle_remaining"] = 0
			module["forced_reason"] = forced_reason
			return


func toggle_at(active_index: int) -> Dictionary:
	var active_count: int = 0
	for entry: Variant in _modules:
		var module: Dictionary = entry as Dictionary
		if not (module.get("is_active_module", false) as bool):
			continue
		if active_count == active_index:
			var kind: String = module.get("kind", "") as String
			return {
				"module_id": module.get("module_id", 0) as int,
				"slot": module.get("slot", "") as String,
				"kind": kind,
				"is_active": module.get("is_active", false) as bool,
				"requires_target": _requires_target(kind),
				"effective_range": effective_range_for_activation(kind, module.get("module_id", 0) as int),
			}
		active_count += 1
	return {}


func weapon_ranges() -> Dictionary:
	var optimal: float = 0.0
	var falloff: float = 0.0
	for entry: Variant in _modules:
		var module: Dictionary = entry as Dictionary
		if not (module.get("is_active", false) as bool):
			continue
		if module.get("kind", "") as String != "Weapon":
			continue
		var stat_delta: Dictionary = module.get("stat_delta", {}) as Dictionary
		optimal += stat_delta.get("weapon_range_add", 0.0) as float
		falloff += stat_delta.get("falloff_range_add", 0.0) as float
	return {"optimal": optimal, "falloff": falloff}


func effective_range_for_activation(kind: String, module_id: int) -> float:
	var family: String = _range_family(kind)
	if family == "":
		return -1.0

	var total: float = 0.0
	for entry: Variant in _modules:
		var module: Dictionary = entry as Dictionary
		var is_this_module: bool = (module.get("module_id", -1) as int) == module_id
		if not is_this_module and not (module.get("is_active", false) as bool):
			continue
		var mkind: String = module.get("kind", "") as String
		if _range_family(mkind) != family:
			continue
		var stat_delta: Dictionary = module.get("stat_delta", {}) as Dictionary
		match family:
			"weapon":
				total += (stat_delta.get("weapon_range_add", 0.0) as float) \
					+ (stat_delta.get("falloff_range_add", 0.0) as float)
			"tackle":
				total += stat_delta.get("tackle_range_add", 0.0) as float
			"repair":
				total += stat_delta.get("repair_range_add", 0.0) as float
	return total


func simulate_capacitor_ticks(cap_current: float, cap_max: float, cap_recharge: float, ticks: int) -> float:
	return simulate_modules_capacitor_ticks(_modules, cap_current, cap_max, cap_recharge, ticks)


static func simulate_modules_capacitor_ticks(
	module_rows: Array,
	cap_current: float,
	cap_max: float,
	cap_recharge: float,
	ticks: int,
) -> float:
	var cap: float = cap_current
	for _cap_tick_step: int in range(ticks):
		cap = minf(cap + cap_recharge, cap_max)
		for entry: Variant in module_rows:
			var module: Dictionary = entry as Dictionary
			if not (module.get("is_active_module", false) as bool):
				continue
			if not (module.get("is_active", false) as bool):
				continue

			var cycle_remaining: int = module.get("cycle_remaining", 0) as int
			var cost: float = module.get("cap_cost_per_cycle", 0.0) as float
			var cycle_ticks: int = module.get("cycle_time_ticks", 10) as int

			if cycle_remaining == 0:
				if cost <= 0.0 or cap >= cost:
					cap -= cost
					module["cycle_remaining"] = cycle_ticks
			else:
				module["cycle_remaining"] = cycle_remaining - 1
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
