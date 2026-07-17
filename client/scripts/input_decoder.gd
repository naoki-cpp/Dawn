## input_decoder.gd
##
## Decides what a keyboard shortcut means given the current selection state,
## without performing any side effect (no network sends, no state mutation).
## Extracted from main.gd's _input() as part of architecture-review-
## client.md C-1's InputHandler slice -- but scoped to just the keyboard
## decision, which is a pure function of (keycode, selection state) and
## therefore testable without a scene tree. The mouse-click flow (double-
## click timing, HUD module-slot hit-testing, picking-to-selection) stays in
## main.gd: it's genuinely stateful (double-click timer) and coupled to
## HUD code that hasn't been extracted yet, so pulling it out now would add
## indirection without removing real coupling.
##
## Returns a Dictionary {"kind": "..."} describing what main.gd should do;
## main.gd's _input() matches on "kind" and performs the actual command
## sends / state writes. {"kind": "none"} means the key had no effect.
class_name InputDecoder
extends RefCounted


## Decides the action for a single keypress. `player_ship_id` < 0 means no
## player ship yet -- F-keys, Tab, and I (inventory panel, ownership.md §8's
## shipless-docked recovery path) still work in that case, mirroring
## main.gd's original per-branch `and _player_ship_id >= 0` guards for
## everything else.
static func decode_key(
	keycode: Key,
	player_ship_id: int,
	selected_gate_id: int,
	selected_target_id: int,
	selected_body_id: int,
	nearby_gate_id: int,
	nearby_station_id: int = -1,
	docked_station_id: int = -1,
) -> Dictionary:
	var f_index: int = _f_key_index(keycode)
	if f_index >= 0:
		return {"kind": "toggle_module", "module_index": f_index}

	## S key -> StopCommand (decelerate to stop using thrust)
	if keycode == KEY_S and player_ship_id >= 0:
		return {"kind": "stop"}

	## J key -> JumpCommand. Explicit gate selection takes priority over
	## proximity detection; if no gate is selected, fall back to the gate in
	## range. If already in range, jump immediately. If a gate is selected
	## but out of range, send JumpCommand anyway -- the server will
	## auto-warp first (ADR-0022).
	if keycode == KEY_J and player_ship_id >= 0:
		var jump_gate: int = selected_gate_id if selected_gate_id >= 0 else nearby_gate_id
		if jump_gate >= 0:
			return {"kind": "jump", "gate_id": jump_gate}
		return {"kind": "none"}

	## A key -> ApproachCommand (auto-approach selected ship/gate, ADR-0015)
	if keycode == KEY_A and player_ship_id >= 0:
		if selected_gate_id >= 0:
			return {"kind": "approach_gate", "gate_id": selected_gate_id}
		if selected_target_id >= 0:
			return {"kind": "approach_ship", "ship_id": selected_target_id}
		return {"kind": "none"}

	## W key -> WarpCommand (short-range Fold to selected gate or body, ADR-0022/ADR-0025)
	if keycode == KEY_W and player_ship_id >= 0:
		if selected_gate_id >= 0:
			return {"kind": "warp_to_gate", "gate_id": selected_gate_id}
		if selected_body_id >= 0:
			return {"kind": "warp_to_body", "body_id": selected_body_id}
		return {"kind": "none"}

	## O key -> OrbitCommand (sweep around selected ship/gate, ADR-0031)
	if keycode == KEY_O and player_ship_id >= 0:
		if selected_gate_id >= 0:
			return {"kind": "orbit_gate", "gate_id": selected_gate_id}
		if selected_target_id >= 0:
			return {"kind": "orbit_ship", "ship_id": selected_target_id}
		return {"kind": "none"}

	## K key -> KeepAtRangeCommand (hold a stand-off from selected ship/gate, ADR-0031)
	if keycode == KEY_K and player_ship_id >= 0:
		if selected_gate_id >= 0:
			return {"kind": "keep_at_range_gate", "gate_id": selected_gate_id}
		if selected_target_id >= 0:
			return {"kind": "keep_at_range_ship", "ship_id": selected_target_id}
		return {"kind": "none"}

	## [ / ] keys -> adjust the stand-off distance the next K press will use.
	## Lets the player dial in "keep at range" before locking it in, since
	## the right distance depends on the target's weapon range and is a
	## tactical choice, not something the server should pick alone.
	if keycode == KEY_BRACKETLEFT and player_ship_id >= 0:
		return {"kind": "adjust_keep_at_range", "delta_km": -1.0}
	if keycode == KEY_BRACKETRIGHT and player_ship_id >= 0:
		return {"kind": "adjust_keep_at_range", "delta_km": 1.0}

	## I key -> toggle the Inventory / Fitting panel (ADR-0032). No
	## player_ship_id requirement, unlike the keys below: a docked player
	## with no active ship (post-Disassemble/Disembark, ownership.md §8)
	## must still be able to open this panel -- it's the only way to reach
	## the STATION column's Assemble row or the SHIPS column's
	## SelectActiveShip rows, both of which are the documented recovery path
	## and neither of which requires an active ship (see main.gd's
	## ACTION_ASSEMBLE/ACTION_SELECT_ACTIVE_SHIP handlers). Gating this key
	## on player_ship_id would strand a shipless docked player: unable to
	## open the panel, unable to Assemble a replacement.
	if keycode == KEY_I:
		return {"kind": "toggle_inventory_panel"}

	## M key -> toggle the Market panel. The server still validates the active
	## ship and item ownership when an order is submitted.
	if keycode == KEY_M:
		return {"kind": "toggle_market_panel"}

	## D key -> dock at the nearby station when in range.
	if keycode == KEY_D and player_ship_id >= 0:
		if nearby_station_id >= 0 and docked_station_id < 0:
			return {"kind": "dock", "station_id": nearby_station_id}
		return {"kind": "none"}

	## U key -> undock from the current station.
	if keycode == KEY_U and player_ship_id >= 0:
		if docked_station_id >= 0:
			return {"kind": "undock"}
		return {"kind": "none"}

	## B key -> build a packaged ship at the current station.
	if keycode == KEY_B and player_ship_id >= 0:
		if docked_station_id >= 0:
			return {"kind": "build_packaged_ship", "station_id": docked_station_id}
		return {"kind": "none"}

	## Y key -> disassemble the currently-docked ship at the current station.
	if keycode == KEY_Y and player_ship_id >= 0:
		if docked_station_id >= 0:
			return {"kind": "disassemble_ship", "station_id": docked_station_id}
		return {"kind": "none"}

	## X key -> disembark: leave the active ship while docked, without
	## disassembling it (ADR-0037).
	if keycode == KEY_X and player_ship_id >= 0:
		if docked_station_id >= 0:
			return {"kind": "disembark"}
		return {"kind": "none"}

	## Tab key -> toggle tactical overlay visibility (works regardless of
	## whether a player ship exists yet).
	if keycode == KEY_TAB:
		return {"kind": "toggle_tactical_overlay"}

	return {"kind": "none"}


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
