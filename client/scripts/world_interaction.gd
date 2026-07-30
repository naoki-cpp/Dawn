## world_interaction.gd
##
## Client-side world interaction policy for one play session. This module owns
## selection state, double-click timing, and the mapping from normalized input
## facts to world intents. main.gd stays responsible for scene wiring, network
## sends, and visual side effects.
class_name WorldInteraction
extends RefCounted

const InputDecoder = preload("res://scripts/input_decoder.gd")

const DOUBLE_CLICK_SEC: float = 0.4
const DOUBLE_CLICK_PX: float = 10.0

var _selected_target_id: int = -1
var _selected_gate_id: int = -1
var _selected_body_id: int = -1
var _last_click_time: float = -1.0
var _last_click_pos: Vector2 = Vector2.ZERO


func selected_target_id() -> int:
	return _selected_target_id


func selected_gate_id() -> int:
	return _selected_gate_id


func selected_body_id() -> int:
	return _selected_body_id


func clear_selection() -> void:
	_selected_target_id = -1
	_selected_gate_id = -1
	_selected_body_id = -1


func clear_navigation_selection() -> void:
	_selected_gate_id = -1
	_selected_body_id = -1


func clear_target_if_matches(ship_id: int) -> void:
	if ship_id == _selected_target_id:
		_selected_target_id = -1


func resolve_key_action(
	keycode: Key,
	player_ship_id: int,
	nearby_gate_id: int,
	nearby_station_id: int,
	docked_station_id: int
) -> Dictionary:
	return InputDecoder.decode_key(
		keycode,
		player_ship_id,
		_selected_gate_id,
		_selected_target_id,
		_selected_body_id,
		nearby_gate_id,
		nearby_station_id,
		docked_station_id
	)


func interpret_primary_click(
	screen_pos: Vector2,
	now_sec: float,
	camera_dragging: bool,
	player_ship_id: int,
	hit_ship_id: int,
	hit_gate_id: int,
	hit_body_id: int
) -> Dictionary:
	if player_ship_id < 0:
		return {"kind": "none"}

	var dt: float = now_sec - _last_click_time
	var dp: float = screen_pos.distance_to(_last_click_pos)
	if dt < DOUBLE_CLICK_SEC and dp < DOUBLE_CLICK_PX:
		_last_click_time = -1.0
		if camera_dragging:
			return {"kind": "none"}
		return {"kind": "double_click_move"}

	_last_click_time = now_sec
	_last_click_pos = screen_pos

	if hit_ship_id >= 0:
		_select_ship(hit_ship_id)
		return {"kind": "selection_changed"}
	if hit_gate_id >= 0:
		_select_gate(hit_gate_id)
		return {"kind": "selection_changed"}
	if hit_body_id >= 0:
		_select_body(hit_body_id)
		return {"kind": "selection_changed"}
	return {"kind": "none"}


func interpret_lock_click(player_ship_id: int, hit_ship_id: int) -> Dictionary:
	if player_ship_id < 0 or hit_ship_id < 0:
		return {"kind": "none"}
	return {"kind": "lock_on", "ship_id": hit_ship_id}


func _select_ship(ship_id: int) -> void:
	_selected_target_id = ship_id
	_selected_gate_id = -1
	_selected_body_id = -1


func _select_gate(gate_id: int) -> void:
	_selected_gate_id = gate_id
	_selected_target_id = -1
	_selected_body_id = -1


func _select_body(body_id: int) -> void:
	_selected_body_id = body_id
	_selected_gate_id = -1
	_selected_target_id = -1
