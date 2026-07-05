## world_session.gd
##
## Client-side live world state for one connected play session.
## main.gd owns scene wiring and input; this module owns the dictionaries and
## invariants that are updated by InitialState and DomainEvents.
extends RefCounted

const PlayerFitting = preload("res://scripts/player_fitting.gd")

var ships: Dictionary = {}
var ship_hp: Dictionary = {}
var opponent_ship_ids: Array = []
var gates: Array = []
var stations: Array = []
var bodies: Array = []
var system_names: Dictionary = {}

var player_ship_id: int = -1
var player_ship_type_name: String = ""
var player_shield: float = -1.0
var player_armor: float = -1.0
var player_hull: float = -1.0
var player_max_shield: float = 500.0
var player_max_armor: float = 300.0
var player_max_hull: float = 200.0
var player_lock_target: int = -1
var current_tick: int = 0
var event_count: int = 0
var current_system_name: String = "Unknown"
var cap_current: float = -1.0
var cap_max: float = 500.0
var cap_recharge: float = 10.0
var docked_station_id: int = -1
var docked_station_name: String = ""
var latest_dock_state_tick: int = -1


static func vec3_from_dict(d: Dictionary, key: String) -> Vector3:
	var v: Dictionary = d.get(key, {}) as Dictionary
	return Vector3(
		v.get("x", 0.0) as float,
		v.get("y", 0.0) as float,
		v.get("z", 0.0) as float)


func reset() -> void:
	ships.clear()
	ship_hp.clear()
	opponent_ship_ids.clear()
	player_ship_id = -1
	player_ship_type_name = ""
	player_shield = -1.0
	player_armor = -1.0
	player_hull = -1.0
	player_max_shield = 500.0
	player_max_armor = 300.0
	player_max_hull = 200.0
	player_lock_target = -1
	current_tick = 0
	event_count = 0
	cap_current = -1.0
	cap_max = 500.0
	cap_recharge = 10.0
	docked_station_id = -1
	docked_station_name = ""
	latest_dock_state_tick = -1


func has_ship(ship_id: int) -> bool:
	return ships.has(ship_id)


func ship_count() -> int:
	return ships.size()


func register_ship(ship_id: int, node: Node3D, ship_data: Dictionary, connection_ship_id: int) -> Dictionary:
	if ships.has(ship_id):
		return {"registered": false, "became_player": false, "became_opponent": false}

	ships[ship_id] = node
	_record_ship_hp(ship_id, ship_data)

	var became_player := false
	var became_opponent := false
	if ship_id == connection_ship_id and player_ship_id < 0:
		_set_player_ship_from_state(ship_id, ship_data)
		became_player = true
	elif ship_data.get("is_player", false) as bool:
		if ship_id not in opponent_ship_ids:
			opponent_ship_ids.append(ship_id)
		became_opponent = true

	return {
		"registered": true,
		"became_player": became_player,
		"became_opponent": became_opponent,
	}


## `clear_lock`: false for an AoI leave (ADR-0019) -- the ship is still alive
## and still Locked server-side (Lock has no distance-based expiry today, only
## despawn/destruction clears a lock, see lock.rs), just outside this
## player's AoI radius. Clearing player_lock_target here would desync from
## the server: a fresh LockOnCommand the player sends afterward is silently
## ignored server-side (it already has_target()==true), so no new
## TargetLocked ever arrives to resync the client -- the lock would look
## like it "never completes" even once the target is back in range/view.
## True despawn/destruction (the ship is actually gone) should still clear it.
func remove_ship(ship_id: int, clear_lock: bool = true) -> Dictionary:
	if not ships.has(ship_id):
		return {"removed": false}
	var node: Node3D = ships[ship_id] as Node3D
	ships.erase(ship_id)
	ship_hp.erase(ship_id)
	var cleared_selection := false
	var removed_player := false
	var removed_opponent := false
	if ship_id == player_ship_id:
		player_ship_id = -1
		removed_player = true
	if ship_id in opponent_ship_ids:
		opponent_ship_ids.erase(ship_id)
		removed_opponent = true
	if clear_lock and ship_id == player_lock_target:
		player_lock_target = -1
	return {
		"removed": true,
		"node": node,
		"removed_player": removed_player,
		"removed_opponent": removed_opponent,
		"cleared_lock": ship_id == player_lock_target,
		"cleared_selection": cleared_selection,
	}


func destroy_ship(ship_id: int) -> Dictionary:
	if not ships.has(ship_id):
		return {"destroyed": false}
	var node: Node3D = ships[ship_id] as Node3D
	ships.erase(ship_id)
	ship_hp.erase(ship_id)
	var destroyed_player := ship_id == player_ship_id
	var destroyed_opponent := ship_id in opponent_ship_ids
	if destroyed_player:
		player_ship_id = -1
		player_shield = 0.0
		player_armor = 0.0
		player_hull = 0.0
		player_lock_target = -1
	if destroyed_opponent:
		opponent_ship_ids.erase(ship_id)
	if ship_id == player_lock_target:
		player_lock_target = -1
	return {
		"destroyed": true,
		"node": node,
		"destroyed_player": destroyed_player,
		"destroyed_opponent": destroyed_opponent,
	}


func ingest_navigation(state: Dictionary) -> void:
	current_system_name = state.get("system_name", current_system_name) as String

	system_names.clear()
	for entry: Variant in (state.get("systems", []) as Array):
		var s: Dictionary = entry as Dictionary
		system_names[s.get("id", -1) as int] = s.get("name", "") as String

	gates.clear()
	for entry: Variant in (state.get("jump_gates", []) as Array):
		var g: Dictionary = entry as Dictionary
		gates.append({
			"gate_id": g.get("gate_id", -1) as int,
			"position": vec3_from_dict(g, "position"),
			"activation_radius": g.get("activation_radius", 0.0) as float,
			"to_system_name": g.get("to_system_name", "") as String,
		})

	stations.clear()
	for entry: Variant in (state.get("stations", []) as Array):
		var station: Dictionary = entry as Dictionary
		stations.append({
			"station_id": station.get("station_id", -1) as int,
			"name": station.get("name", "") as String,
			"position": vec3_from_dict(station, "position"),
			"docking_radius": station.get("docking_radius", 0.0) as float,
		})

	bodies.clear()
	for entry: Variant in (state.get("celestial_bodies", []) as Array):
		var b: Dictionary = entry as Dictionary
		bodies.append({
			"body_id": b.get("id", -1) as int,
			"kind": b.get("kind", "") as String,
			"name": b.get("name", "") as String,
			"position": vec3_from_dict(b, "position"),
			"radius": b.get("radius", 1.0) as float,
			"spectral_type": b.get("spectral_type", 0.0) as float,
		})


func system_changed(ship_id: int, to_system: int) -> Dictionary:
	if ship_id != player_ship_id:
		return {"changed_player": false}
	current_system_name = system_names.get(to_system, "System %d" % to_system) as String
	return {"changed_player": true, "system_name": current_system_name}


func apply_hp_event(payload: Dictionary) -> Dictionary:
	var ship_id: int = payload.get("ship_id", 0) as int
	var entry: Dictionary = ship_hp.get(ship_id, {}) as Dictionary
	entry["shield"] = payload.get("current_shield", 0.0) as float
	entry["armor"] = payload.get("current_armor", 0.0) as float
	entry["hull"] = payload.get("current_hull", 0.0) as float
	ship_hp[ship_id] = entry
	var changed_player := ship_id == player_ship_id
	if changed_player:
		player_shield = entry["shield"] as float
		player_armor = entry["armor"] as float
		player_hull = entry["hull"] as float
	return {"ship_id": ship_id, "changed_player": changed_player, "has_ship": ships.has(ship_id)}


func apply_target_locked(locker_id: int, target_id: int) -> bool:
	if locker_id != player_ship_id:
		return false
	player_lock_target = target_id
	return true


func apply_lock_lost(locker_id: int, target_id: int) -> bool:
	if locker_id != player_ship_id:
		return false
	if target_id == player_lock_target:
		player_lock_target = -1
	return true


func advance_tick_from_event(tick: int, modules: Array) -> int:
	if tick <= current_tick:
		return 0
	var ticks_elapsed: int = tick - current_tick
	current_tick = tick
	simulate_cap(ticks_elapsed, modules)
	return ticks_elapsed


func advance_client_ticks(ticks: int, modules: Array) -> void:
	if ticks <= 0:
		return
	current_tick += ticks
	simulate_cap(ticks, modules)


func simulate_cap(ticks: int, modules: Array) -> void:
	if cap_current < 0.0 or player_ship_id < 0:
		return
	cap_current = PlayerFitting.simulate_capacitor_ticks(
		modules,
		cap_current,
		cap_max,
		cap_recharge,
		ticks)


func apply_dock_event(ship_id: int, station_id: int, station_name: String, tick: int) -> bool:
	if ship_id != player_ship_id:
		return false
	return _apply_dock_state(station_id, station_name, tick)


func apply_undock_event(ship_id: int, tick: int) -> bool:
	if ship_id != player_ship_id:
		return false
	return _apply_dock_state(-1, "", tick)


func apply_dock_fitting(station_id: int, station_name: String, tick: int) -> bool:
	return _apply_dock_state(station_id, station_name, tick)


func dock_status() -> Dictionary:
	return {
		"docked_station_id": docked_station_id,
		"docked_station_name": docked_station_name,
		"is_docked": docked_station_id >= 0,
		"latest_dock_state_tick": latest_dock_state_tick,
	}


func is_docked() -> bool:
	return docked_station_id >= 0


func _apply_dock_state(station_id: int, station_name: String, tick: int) -> bool:
	if tick < latest_dock_state_tick:
		return false
	latest_dock_state_tick = tick
	docked_station_id = station_id
	docked_station_name = station_name
	return true


func target_hp() -> Dictionary:
	return ship_hp.get(player_lock_target, {}) as Dictionary


func _record_ship_hp(ship_id: int, ship_data: Dictionary) -> void:
	var max_shield: float = ship_data.get("max_shield", 200.0) as float
	var max_armor: float = ship_data.get("max_armor", 150.0) as float
	var max_hull: float = ship_data.get("max_hull", 150.0) as float
	ship_hp[ship_id] = {
		"shield": ship_data.get("current_shield", max_shield) as float,
		"armor": ship_data.get("current_armor", max_armor) as float,
		"hull": ship_data.get("current_hull", max_hull) as float,
		"max_shield": max_shield,
		"max_armor": max_armor,
		"max_hull": max_hull,
	}


func _set_player_ship_from_state(ship_id: int, ship_data: Dictionary) -> void:
	var hp: Dictionary = ship_hp[ship_id] as Dictionary
	player_ship_id = ship_id
	player_max_shield = hp["max_shield"] as float
	player_max_armor = hp["max_armor"] as float
	player_max_hull = hp["max_hull"] as float
	player_shield = hp["shield"] as float
	player_armor = hp["armor"] as float
	player_hull = hp["hull"] as float
	cap_max = ship_data.get("cap_max", 500.0) as float
	cap_recharge = ship_data.get("cap_recharge_per_tick", 10.0) as float
	cap_current = cap_max
	player_ship_type_name = ship_data.get("ship_type_name", "") as String
