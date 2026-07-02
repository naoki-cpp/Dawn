## player_fitting.gd
##
## Normalizes the PlayerFitting wire message into the client-side shape used by
## HUD, module toggles, and capacitor simulation. The server still owns the
## schema; this module is the client seam that keeps raw Dictionary keys local.
extends RefCounted


static func normalize_payload(payload: Dictionary) -> Dictionary:
	var modules: Array = []
	for entry: Variant in payload.get("modules", []) as Array:
		var src: Dictionary = entry as Dictionary
		var stat_delta: Dictionary = src.get("stat_delta", {}) as Dictionary
		modules.append({
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
			},
		})

	var inventory: Array = []
	for entry: Variant in payload.get("inventory", []) as Array:
		var src: Dictionary = entry as Dictionary
		inventory.append({
			"module_id": src.get("module_id", 0) as int,
			"name": src.get("name", "?") as String,
			"kind": src.get("kind", "") as String,
			"slot": src.get("slot", "") as String,
		})

	return {
		"modules": modules,
		"inventory": inventory,
		"slot_capacity": payload.get("slot_capacity", {}) as Dictionary,
	}


static func weapon_ranges(modules: Array) -> Dictionary:
	var optimal: float = 0.0
	var falloff: float = 0.0
	for entry: Variant in modules:
		var module: Dictionary = entry as Dictionary
		if not (module.get("is_active", false) as bool):
			continue
		if module.get("kind", "") as String != "Weapon":
			continue
		var stat_delta: Dictionary = module.get("stat_delta", {}) as Dictionary
		optimal += stat_delta.get("weapon_range_add", 0.0) as float
		falloff += stat_delta.get("falloff_range_add", 0.0) as float
	return {"optimal": optimal, "falloff": falloff}


## p_forced_reason: "cap" | "range" | "" (server-authoritative reason for a
## non-player-issued deactivation, ADR-0035; "" for ON or a player-issued OFF).
static func set_module_activation(
	modules: Array,
	module_id: int,
	active: bool,
	p_forced_reason: String = "",
) -> void:
	for entry: Variant in modules:
		var module: Dictionary = entry as Dictionary
		if module.get("module_id", 0) as int == module_id:
			module["is_active"] = active
			module["cycle_remaining"] = 0
			module["forced_reason"] = p_forced_reason
			return


static func active_module_toggle_at(modules: Array, active_index: int) -> Dictionary:
	var active_count: int = 0
	for entry: Variant in modules:
		var module: Dictionary = entry as Dictionary
		if not (module.get("is_active_module", false) as bool):
			continue
		if active_count == active_index:
			return {
				"module_id": module.get("module_id", 0) as int,
				"slot": module.get("slot", "") as String,
				"is_active": module.get("is_active", false) as bool,
				"kind": module.get("kind", "") as String,
			}
		active_count += 1
	return {}


static func simulate_capacitor_ticks(
	modules: Array,
	cap_current: float,
	cap_max: float,
	cap_recharge: float,
	ticks: int,
) -> float:
	var cap: float = cap_current
	for _tick: int in range(ticks):
		cap = minf(cap + cap_recharge, cap_max)
		for entry: Variant in modules:
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
