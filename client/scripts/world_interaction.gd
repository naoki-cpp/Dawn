##
## Client-side world interaction policy for one play session. This module owns
## typed selection state, double-click timing, and the mapping from normalized
## input facts to ClientIntent objects. main.gd stays responsible for scene
## wiring, network sends, and visual side effects.
class_name WorldInteraction
extends RefCounted

const InputDecoder = preload("res://scripts/input_decoder.gd")

const DOUBLE_CLICK_SEC: float = 0.4
const DOUBLE_CLICK_PX: float = 10.0

var _selection: ClientSelection = ClientSelection.none()
var _last_click_time: float = -1.0
var _last_click_pos: Vector2 = Vector2.ZERO


## Scalar accessors remain for presentation code that needs to compare a
## selected ID. The source of truth is the mutually exclusive ClientSelection
## object above, not three independently mutable integer fields.
func selected_target_id() -> int:
	return _selection.id() if _selection.is_ship() else -1


func selected_gate_id() -> int:
	return _selection.id() if _selection.is_gate() else -1


func selected_body_id() -> int:
	return _selection.id() if _selection.is_body() else -1


func selected_station_id() -> int:
	return _selection.id() if _selection.is_station() else -1


func clear_selection() -> void:
	_selection = ClientSelection.none()


func clear_navigation_selection() -> void:
	if _selection.is_gate() or _selection.is_body() or _selection.is_station():
		_selection = ClientSelection.none()


func clear_target_if_matches(ship_id: int) -> void:
	if _selection.is_ship() and ship_id == _selection.id():
		_selection = ClientSelection.none()


func resolve_key_intent(
	keycode: Key,
	player_ship_id: int,
	nearby_gate_id: int,
	nearby_station_id: int,
	docked_station_id: int
) -> ClientIntent:
	return InputDecoder.decode_key(
		keycode,
		player_ship_id,
		_selection,
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
	hit_body_id: int,
	hit_station_id: int = -1
) -> ClientIntent:
	if player_ship_id < 0:
		return ClientIntent.none()

	var dt: float = now_sec - _last_click_time
	var dp: float = screen_pos.distance_to(_last_click_pos)
	if dt < DOUBLE_CLICK_SEC and dp < DOUBLE_CLICK_PX:
		_last_click_time = -1.0
		if camera_dragging:
			return ClientIntent.none()
		return ClientIntent.double_click_move()

	_last_click_time = now_sec
	_last_click_pos = screen_pos

	if hit_ship_id >= 0:
		_selection = ClientSelection.ship(hit_ship_id)
		return ClientIntent.selection_changed()
	if hit_gate_id >= 0:
		_selection = ClientSelection.gate(hit_gate_id)
		return ClientIntent.selection_changed()
	if hit_station_id >= 0:
		_selection = ClientSelection.station(hit_station_id)
		return ClientIntent.selection_changed()
	if hit_body_id >= 0:
		_selection = ClientSelection.body(hit_body_id)
		return ClientIntent.selection_changed()
	return ClientIntent.none()


func interpret_lock_click(player_ship_id: int, hit_ship_id: int) -> ClientIntent:
	if player_ship_id < 0 or hit_ship_id < 0:
		return ClientIntent.none()
	return ClientIntent.lock_on(hit_ship_id)
