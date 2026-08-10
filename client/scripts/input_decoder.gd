##
## Decides what a keyboard shortcut means given the current selection state,
## without performing any side effect (no network sends, no state mutation).
##
## The result is a typed ClientIntent object. The caller never parses string
## tags or reads payloads through magic Dictionary keys.
class_name InputDecoder
extends RefCounted


## Decides the intent for a single keypress. `player_ship_id` < 0 means no
## player ship yet. Inventory, tactical overlay, and F-keys retain their
## shipless behavior so a docked player can recover after disembarking.
static func decode_key(
	keycode: Key,
	player_ship_id: int,
	selection: ClientSelection,
	nearby_gate_id: int,
	nearby_station_id: int = -1,
	docked_station_id: int = -1,
) -> ClientIntent:
	var f_index: int = _f_key_index(keycode)
	if f_index >= 0:
		return ClientIntent.toggle_module(f_index)

	if keycode == KEY_S and player_ship_id >= 0:
		return ClientIntent.stop()

	## Explicit gate selection takes priority over proximity detection.
	if keycode == KEY_J and player_ship_id >= 0:
		var jump_gate: int = selection.id() if selection.is_gate() else nearby_gate_id
		if jump_gate >= 0:
			return ClientIntent.jump(jump_gate)
		return ClientIntent.none()

	if keycode == KEY_A and player_ship_id >= 0:
		if selection.is_gate():
			return ClientIntent.approach_gate(selection.id())
		if selection.is_ship():
			return ClientIntent.approach_ship(selection.id())
		return ClientIntent.none()

	if keycode == KEY_W and player_ship_id >= 0:
		if selection.is_gate():
			return ClientIntent.warp_to_gate(selection.id())
		if selection.is_body():
			return ClientIntent.warp_to_body(selection.id())
		return ClientIntent.none()

	if keycode == KEY_O and player_ship_id >= 0:
		if selection.is_gate():
			return ClientIntent.orbit_gate(selection.id())
		if selection.is_ship():
			return ClientIntent.orbit_ship(selection.id())
		return ClientIntent.none()

	if keycode == KEY_K and player_ship_id >= 0:
		if selection.is_gate():
			return ClientIntent.keep_at_range_gate(selection.id())
		if selection.is_ship():
			return ClientIntent.keep_at_range_ship(selection.id())
		return ClientIntent.none()

	if keycode == KEY_BRACKETLEFT and player_ship_id >= 0:
		return ClientIntent.adjust_keep_at_range(-1.0)
	if keycode == KEY_BRACKETRIGHT and player_ship_id >= 0:
		return ClientIntent.adjust_keep_at_range(1.0)

	## A shipless docked player must still be able to open the inventory panel
	## and recover through Assemble or SelectActiveShip.
	if keycode == KEY_I:
		return ClientIntent.toggle_inventory_panel()

	if keycode == KEY_M and docked_station_id >= 0:
		return ClientIntent.toggle_market_panel()

	if keycode == KEY_D and player_ship_id >= 0:
		if nearby_station_id >= 0 and docked_station_id < 0:
			return ClientIntent.dock(nearby_station_id)
		return ClientIntent.none()

	if keycode == KEY_U and player_ship_id >= 0:
		if docked_station_id >= 0:
			return ClientIntent.undock()
		return ClientIntent.none()

	if keycode == KEY_B and player_ship_id >= 0:
		if docked_station_id >= 0:
			return ClientIntent.build_packaged_ship(docked_station_id)
		return ClientIntent.none()

	if keycode == KEY_Y and player_ship_id >= 0:
		if docked_station_id >= 0:
			return ClientIntent.disassemble_ship(docked_station_id)
		return ClientIntent.none()

	if keycode == KEY_X and player_ship_id >= 0:
		if docked_station_id >= 0:
			return ClientIntent.disembark()
		return ClientIntent.none()

	if keycode == KEY_TAB:
		return ClientIntent.toggle_tactical_overlay()

	return ClientIntent.none()


static func _f_key_index(keycode: Key) -> int:
	match keycode:
		KEY_F1: return 0
		KEY_F2: return 1
		KEY_F3: return 2
		KEY_F4: return 3
		KEY_F5: return 4
		KEY_F6: return 5
		KEY_F7: return 6
		KEY_F8: return 7
	return -1
