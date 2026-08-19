##
## Godot adapter for the engine-independent ClientInteraction policy.
##
## This file only normalizes Godot key constants and hit-test results. Selection
## state, double-click timing, and action construction live in dawn-client-core.
class_name WorldInteraction
extends RefCounted

const ACTION_NONE: int = 0
const ACTION_REQUEST: int = 1
const ACTION_LOCAL: int = 2

const LOCAL_NONE: int = 0
const LOCAL_TOGGLE_MODULE: int = 1
const LOCAL_ADJUST_KEEP_AT_RANGE: int = 2
const LOCAL_TOGGLE_INVENTORY: int = 3
const LOCAL_TOGGLE_MARKET: int = 4
const LOCAL_TOGGLE_TACTICAL_OVERLAY: int = 5
const LOCAL_DOUBLE_CLICK_MOVE: int = 6
const LOCAL_SELECTION_CHANGED: int = 7

var _core: ClientInteraction = ClientInteraction.new()


func selected_target_id() -> int:
	return _core.selected_target_id()


func selected_gate_id() -> int:
	return _core.selected_gate_id()


func selected_body_id() -> int:
	return _core.selected_body_id()


func selected_station_id() -> int:
	return _core.selected_station_id()


func clear_selection() -> void:
	_core.clear_selection()


func clear_navigation_selection() -> void:
	_core.clear_navigation_selection()


func clear_target_if_matches(ship_id: int) -> void:
	_core.clear_target_if_matches(ship_id)


func resolve_key_action(
	keycode: Key,
	player_ship_id: int,
	nearby_gate_id: int,
	nearby_station_id: int,
	docked_station_id: int,
	keep_at_range_m: float,
	buildable_ship_type_id: int
) -> ClientAction:
	return _core.resolve_key_action(
		_normalize_key(keycode),
		player_ship_id,
		nearby_gate_id,
		nearby_station_id,
		docked_station_id,
		keep_at_range_m,
		buildable_ship_type_id)


func interpret_primary_click(
	screen_pos: Vector2,
	now_sec: float,
	camera_dragging: bool,
	player_ship_id: int,
	hit_ship_id: int,
	hit_gate_id: int,
	hit_body_id: int,
	hit_station_id: int = -1
) -> ClientAction:
	return _core.primary_click(
		screen_pos,
		now_sec,
		camera_dragging,
		player_ship_id,
		hit_ship_id,
		hit_gate_id,
		hit_body_id,
		hit_station_id)


func interpret_lock_click(player_ship_id: int, hit_ship_id: int) -> ClientAction:
	return _core.lock_click(player_ship_id, hit_ship_id)


## Godot's Key enum is an engine concern. The resulting small integer is the
## stable adapter protocol consumed by dawn-client-core::ClientKey.
static func _normalize_key(keycode: Key) -> int:
	match keycode:
		KEY_F1: return 1
		KEY_F2: return 2
		KEY_F3: return 3
		KEY_F4: return 4
		KEY_F5: return 5
		KEY_F6: return 6
		KEY_F7: return 7
		KEY_F8: return 8
		KEY_S: return 9
		KEY_J: return 10
		KEY_A: return 11
		KEY_W: return 12
		KEY_O: return 13
		KEY_K: return 14
		KEY_BRACKETLEFT: return 15
		KEY_BRACKETRIGHT: return 16
		KEY_I: return 17
		KEY_M: return 18
		KEY_D: return 19
		KEY_U: return 20
		KEY_B: return 21
		KEY_Y: return 22
		KEY_X: return 23
		KEY_TAB: return 24
	return 0
