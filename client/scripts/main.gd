## main.gd
##
## Root script for the main scene.
##
## Cycle 2:
##   - Left-drag rotates camera (distinguished from click)
##   - Left double-click -> MoveCommand (set acceleration vector)
##   - Designate first Ship as player ship -> send set_player_ship signal to server
##   - Player ship rendered in orange

extends Node

# -- Node references ----------------------------------------------------------

@onready var _connection   : Node        = $Connection
@onready var _ships_root   : Node3D      = $World/Ships
@onready var _gates_root   : Node3D      = $World/Gates
@onready var _bodies_root  : Node3D      = $World/Bodies
@onready var _stats_label  : Label       = $HUD/StatsLabel
@onready var _hud          : CanvasLayer = $HUD
@onready var _camera       : Camera3D    = $World/Camera3D
@onready var _directional_light : DirectionalLight3D = $World/DirectionalLight3D
@onready var _warp_tunnel  : ColorRect   = $HUD/WarpTunnel

# -- Constants ----------------------------------------------------------------

const SHIP_SCENE  := preload("res://scenes/ship.tscn")
const HudSurfaceScript = preload("res://scripts/hud_surface.gd")
const MarketSurfaceScript = preload("res://scripts/market_surface.gd")
const WorldPresentationScript = preload("res://scripts/world_presentation.gd")
const WorldInteractionScript = preload("res://scripts/world_interaction.gd")
const RadialMenuScript = preload("res://scripts/radial_menu.gd")
const InventoryRow = preload("res://scripts/inventory_row.gd")
const CLIENT_TICKS_PER_SEC : float = 10.0
const WORLD_RADIAL_HOLD_SEC: float = 0.36

const BUILDABLE_SHIP_TYPE_ID : int = 7

var _cap_tick_accumulator : float = 0.0

# -- Internal state -----------------------------------------------------------

var _session := WorldSession.new()
var _interaction := WorldInteractionScript.new()
var _station_inventory := StationInventoryInteraction.new()
var _hud_read_model := HudReadModel.new()
var _hud_surface := HudSurfaceScript.new()
var _market_surface := MarketSurfaceScript.new()
var _presentation := WorldPresentationScript.new()
## Scene nodes stay in Godot; WorldSession owns the matching pure ship state.
var _ships                 : Dictionary = {}
## GDScript-owned optimistic state (ADR-0046): both are also mirrored inside
## WorldSession, but main.gd writes these two directly ahead of the server's
## confirming event for immediate UI feedback (_set_as_player_ship,
## _handle_target_locked), so they don't read through _session like every
## other field below did before this cleanup.
var _player_ship_id        : int        = -1
var _player_lock_target : int  = -1

## Stand-off distance (km) the next K-key press will send as KeepAtRangeCommand's
## range. Player-adjustable via [ / ] (ADR-0031) -- the right distance depends
## on the target's weapon range, which is a tactical call the player should
## make, not a value the server should silently pick.
var _keep_at_range_km : float = 10.0
const KEEP_AT_RANGE_MIN_KM : float = 1.0
const KEEP_AT_RANGE_MAX_KM : float = 200.0

## Inventory-panel drag-and-drop (hand-rolled, consistent with the rest of
## this HUD's manual _input() hit-testing -- no native Control drag API).
## `_drag_row` is set on a left-press over a row and cleared on release;
## while non-null, a press+move past DRAG_THRESHOLD_PX is a drop instead of
## a click, resolved against whichever inventory-panel column is under the
## release position (HudHitTest.column_at()).
var _drag_row : InventoryRow = null
var _drag_start_pos : Vector2 = Vector2.ZERO
var _drag_ghost : Label = null
const DRAG_THRESHOLD_PX : float = 6.0

var _world_press_pending: bool = false
var _world_press_started_sec: float = 0.0
var _world_press_position: Vector2 = Vector2.ZERO
var _world_press_target: Dictionary = {}
var _radial_open: bool = false
var _radial_menu: Control = null

## PlayerLoadout is a GDExtension class (dawn-client-gdext, ADR-0039/ADR-0040)
## -- no preload needed, same as any other globally registered class.
var _loadout := PlayerLoadout.new()

var _weapon_range     : float  = 0.0   ## optimal range (u), recalculated on fitting change
var _weapon_falloff   : float  = 0.0   ## falloff range (u)

## Ship HP, the capacitor status, and the current system name live only in
## WorldSession (ADR-0046) -- read via _session.ship_health(id)/
## .capacitor_status()/.current_system_name() at point of use. They change
## on essentially every event, so caching them would just reintroduce the
## staleness risk this cleanup removed.
##
## The navigation map (gates/stations/bodies/buildable ship types) is
## different: it's write-once per Sector, changing only inside
## _ingest_star_map() below. _process() reads gates/stations/bodies every
## frame (proximity checks, presentation refresh), so calling the
## GDExtension accessor there would rebuild these collections from Rust
## state 60x/sec for data that never changes between InitialState messages.
## Cached here instead, refreshed only where _ingest_star_map() writes it.
var _gates                 : Array = []
var _stations              : Array = []
var _bodies                : Array = []
var _buildable_ship_types  : Array = []
var _nearby_gate_id      : int    = -1  ## -1 = no gate in range
var _nearby_station_ids  : Array[int] = []  ## in-range stations, nearest first
var _jump_notice         : String = ""
var _jump_notice_timer   : float  = 0.0
## Warp arrival is handled authoritatively by the server (ADR-0029 warp-arrival
## authority): on every warp completion the server sends a PositionSnap, which
## _handle_position_snap applies. The client no longer pre-computes an arrival
## point or detects arrival from the velocity dropping.

# -- Lifecycle ----------------------------------------------------------------

## Fail fast if main.tscn's node layout drifts from the `@onready` paths above
## (architecture-review/client.md C-3): a missing node currently surfaces as a
## confusing null-deref deep inside whichever method touches it first. Checking
## all of them up front turns that into one clear, actionable error at startup.
func _assert_scene_tree_refs() -> void:
	var missing: Array[String] = []
	if _connection   == null: missing.append("Connection ($Connection)")
	if _ships_root   == null: missing.append("Ships root ($World/Ships)")
	if _gates_root   == null: missing.append("Gates root ($World/Gates)")
	if _bodies_root  == null: missing.append("Bodies root ($World/Bodies)")
	if _stats_label  == null: missing.append("Stats label ($HUD/StatsLabel)")
	if _hud          == null: missing.append("HUD ($HUD)")
	if _camera       == null: missing.append("Camera ($World/Camera3D)")
	if _directional_light == null: missing.append("Directional light ($World/DirectionalLight3D)")
	if _warp_tunnel  == null: missing.append("Warp tunnel ($HUD/WarpTunnel)")
	if not missing.is_empty():
		push_error("main.tscn is missing expected node(s): %s. Check the scene tree against the @onready paths in main.gd." % ", ".join(missing))

var _connection_dialog: CanvasLayer

func _ready() -> void:
	_assert_scene_tree_refs()
	_connection.bind_client_state(_session, _loadout, self)
	_connection.connection_changed.connect(_on_connection_changed)
	_presentation.build(self, _camera, _warp_tunnel, _gates_root, _bodies_root, _world, _directional_light)
	_hud_surface.build(self, _hud, _stats_label)
	_radial_menu = RadialMenuScript.new()
	_radial_menu.z_index = 100
	_hud.add_child(_radial_menu)
	_market_surface.build(
		_hud,
		Callable(self, "_on_market_refresh"),
		Callable(self, "_on_market_place_order"),
		Callable(self, "_on_market_cancel_order"))
	_update_hud()
	## Gate / body markers are spawned from the server's InitialState, not here.
	_connection_dialog = preload("res://scripts/connection_dialog.gd").new()
	add_child(_connection_dialog)
	_connection_dialog.blocking_changed.connect(func(blocking: bool) -> void:
		_camera.set_process_unhandled_input(not blocking))
	_connection_dialog.build(_connection)

func _process(delta: float) -> void:
	_update_radial_hold()
	if _radial_open:
		if not _world_target_exists(_world_press_target):
			_cancel_radial_menu()
		else:
			_radial_menu.call("update_pointer", get_viewport().get_mouse_position())
	_presentation.refresh(delta, _player_ship_id, _ships, _bodies)
	_update_gate_proximity()
	_update_station_proximity()
	_advance_client_cap_ticks(delta)
	if _jump_notice_timer > 0.0:
		_jump_notice_timer -= delta
		if _jump_notice_timer <= 0.0:
			_jump_notice = ""
	_update_hud()

## The client's single coordinate authority (ADR-0029 #3): owns the floating
## origin and is the only place server<->Godot conversions happen. WorldSpace
## is provided by dawn-client-gdext; its coordinate math stays in Rust.
var _world := WorldSpace.new()


## Tracks whether the player ship is within activation range of a Jump Gate
## (ADR-0009). Distance is computed in server units by WorldSpace.
func _update_gate_proximity() -> void:
	_nearby_gate_id = -1
	if _player_ship_id < 0 or not _ships.has(_player_ship_id):
		return
	var ship_pos := _world.to_server_components((_ships[_player_ship_id] as Node3D).global_position)
	for entry: Variant in _gates:
		var gate: GateRecord = entry as GateRecord
		var gate_pos := gate.position
		if _world.distance_components(ship_pos, gate_pos) <= gate.activation_radius:
			_nearby_gate_id = gate.gate_id
			return


## Every station currently within docking range, nearest first. Usually at
## most one (stations shouldn't be placed close enough to overlap docking
## radii), but ranked by distance rather than array order in case they ever
## are, so [D] docks at the one the player is actually closest to.
func _update_station_proximity() -> void:
	_nearby_station_ids.clear()
	if _player_ship_id < 0 or not _ships.has(_player_ship_id):
		return
	var ship_pos := _world.to_server_components((_ships[_player_ship_id] as Node3D).global_position)
	var in_range: Array[Dictionary] = []
	for entry: Variant in _stations:
		var station: StationRecord = entry as StationRecord
		var station_pos := station.position
		var dist: float = _world.distance_components(ship_pos, station_pos)
		if dist <= station.docking_radius:
			in_range.append({"station_id": station.station_id, "distance": dist})
	in_range.sort_custom(func(a: Dictionary, b: Dictionary) -> bool:
		return (a.distance as float) < (b.distance as float))
	for entry: Dictionary in in_range:
		_nearby_station_ids.append(entry.station_id as int)


func _input(event: InputEvent) -> void:
	if _connection_dialog != null and _connection_dialog.is_blocking():
		return
	## Input meaning is decided by the engine-independent client policy. This
	## adapter only performs the resulting network or presentation effect.
	if event is InputEventKey and event.pressed and not event.echo:
		var key: InputEventKey = event as InputEventKey
		if _radial_open:
			if key.keycode == KEY_ESCAPE:
				_cancel_radial_menu()
			return
		if _market_surface.keyboard_consumes():
			return
		var nearest_station_id: int = _nearby_station_ids[0] if not _nearby_station_ids.is_empty() else -1
		var action: ClientAction = _interaction.resolve_key_action(
			key.keycode,
			_player_ship_id,
			_nearby_gate_id,
			nearest_station_id,
			_session.docked_station_id(),
			_keep_at_range_km * 1000.0,
			BUILDABLE_SHIP_TYPE_ID)
		_execute_client_action(action)
		return

	if event is InputEventMouseMotion:
		var motion: InputEventMouseMotion = event as InputEventMouseMotion
		if _radial_open:
			_radial_menu.call("update_pointer", motion.position)
			return
		if _drag_row != null:
			_update_drag_ghost(motion.position)
			return

	if event is InputEventMouseButton:
		var mb: InputEventMouseButton = event as InputEventMouseButton
		if mb.button_index == MOUSE_BUTTON_LEFT and not mb.pressed:
			if _drag_row != null:
				_end_inventory_drag(mb.position)
				return
			if _radial_open:
				_finish_radial_menu(mb.position)
				return
			if _world_press_pending:
				_finish_world_click()
				return
			_camera.call("end_orbit_drag")
			return
		if mb.pressed:
			if _radial_open:
				return
			if _market_surface.panel_consumes(mb.position):
				return
			## A click on the open inventory panel fits/unfits (row hit) or is
			## swallowed (margin/header) -- never a world click. Checked first
			## since the panel can be open over anything.
			if _hud_surface.inventory_panel_consumes(mb.position):
				var inv_row: InventoryRow = _hud_surface.inventory_panel_row_at(mb.position)
				if inv_row != null:
					if mb.button_index == MOUSE_BUTTON_LEFT:
						## Defer to release: a plain click and the start of a
						## drag look identical at press time. See
						## _end_inventory_drag()'s threshold check.
						_drag_row = inv_row
						_drag_start_pos = mb.position
					elif mb.button_index == MOUSE_BUTTON_RIGHT:
						_handle_inventory_row_right_click(inv_row)
				return
			## A click on a module slot toggles it; it is never a world click.
			var slot_index: int = _hud_surface.module_slot_at(mb.position)
			if slot_index >= 0:
				if mb.button_index == MOUSE_BUTTON_LEFT:
					_toggle_module_by_index(slot_index)
				return
			match mb.button_index:
				MOUSE_BUTTON_LEFT:
					_camera.call("begin_orbit_drag", mb.position)
					_begin_world_press(mb.position)
				MOUSE_BUTTON_RIGHT:
					var lock_action: ClientAction = _interaction.interpret_lock_click(
						_player_ship_id,
						_pick_ship_at(mb.position))
					if lock_action.kind() == WorldInteractionScript.ACTION_REQUEST:
						_try_lock_on(lock_action)


func _execute_client_action(action: ClientAction) -> void:
	if action.kind() == WorldInteractionScript.ACTION_REQUEST:
		_connection.send_action(action)
		if action.is_stop_request():
			_apply_stop_prediction()
		return
	if action.kind() != WorldInteractionScript.ACTION_LOCAL:
		return
	match action.local_kind():
		WorldInteractionScript.LOCAL_TOGGLE_MODULE:
			_toggle_module_by_index(action.module_index())
		WorldInteractionScript.LOCAL_ADJUST_KEEP_AT_RANGE:
			_keep_at_range_km = clampf(
				_keep_at_range_km + action.delta_km(),
				KEEP_AT_RANGE_MIN_KM, KEEP_AT_RANGE_MAX_KM)
			_update_hud()
		WorldInteractionScript.LOCAL_TOGGLE_TACTICAL_OVERLAY:
			_presentation.toggle_tactical_overlay()
		WorldInteractionScript.LOCAL_TOGGLE_INVENTORY:
			_hud_surface.toggle_inventory_panel()
		WorldInteractionScript.LOCAL_TOGGLE_MARKET:
			if _market_surface.toggle():
				_connection.send_market_refresh_command()
		WorldInteractionScript.LOCAL_DOUBLE_CLICK_MOVE:
			_on_double_click(action.screen_position())
		WorldInteractionScript.LOCAL_SELECTION_CHANGED:
			_update_hud()

# -- Ship picking (screen position -> nearest ship ID) ------------------------
#
# Picking math lives in ship_picking.gd (ShipPicking, architecture-review/
# client.md C-1) -- main.gd only supplies the live candidate data.

## Returns the ship_id whose node is closest to the click ray (within
## ShipPicking.PICK_RADIUS_SHIP Godot units), excluding the player's own
## ship. -1 if nothing is hit.
func _pick_ship_at(screen_pos: Vector2) -> int:
	if _player_ship_id < 0:
		return -1
	return ShipPicking.pick_ship_at(_camera, screen_pos, _ships, _player_ship_id)

# -- Left-click -> select approach target (ADR-0015) -------------------------

## Returns the gate_id of the Jump Gate (in the current system) whose marker
## is closest to the click ray, or -1. Gates are large objects, so the pick
## radius is wider than for ships.
func _pick_gate_at(screen_pos: Vector2) -> int:
	if _player_ship_id < 0:
		return -1
	return ShipPicking.pick_gate_at(_camera, screen_pos, _gates_root)

## Returns the body_id of the celestial body marker closest to the click
## position on screen, or -1.
func _pick_body_at(screen_pos: Vector2) -> int:
	if _player_ship_id < 0:
		return -1
	return ShipPicking.pick_body_at(_camera, screen_pos, _bodies_root)

## Returns the station_id of the station marker closest to the click position.
func _pick_station_at(screen_pos: Vector2) -> int:
	if _player_ship_id < 0:
		return -1
	return ShipPicking.pick_station_at(_camera, screen_pos, _bodies_root)


func _begin_world_press(screen_pos: Vector2) -> void:
	_world_press_pending = true
	_world_press_started_sec = float(Time.get_ticks_msec()) / 1000.0
	_world_press_position = screen_pos
	_world_press_target = _pick_world_target(screen_pos)


func _pick_world_target(screen_pos: Vector2) -> Dictionary:
	var hit_ship_id := _pick_ship_at(screen_pos)
	if hit_ship_id >= 0:
		return {"ship_id": hit_ship_id, "gate_id": -1, "body_id": -1, "station_id": -1}
	var hit_gate_id := _pick_gate_at(screen_pos)
	if hit_gate_id >= 0:
		return {"ship_id": -1, "gate_id": hit_gate_id, "body_id": -1, "station_id": -1}
	var hit_station_id := _pick_station_at(screen_pos)
	if hit_station_id >= 0:
		return {"ship_id": -1, "gate_id": -1, "body_id": -1, "station_id": hit_station_id}
	var hit_body_id := _pick_body_at(screen_pos)
	return {"ship_id": -1, "gate_id": -1, "body_id": hit_body_id, "station_id": -1}


func _target_kind(target: Dictionary) -> String:
	if int(target.get("ship_id", -1)) >= 0:
		return "ship"
	if int(target.get("gate_id", -1)) >= 0:
		return "gate"
	if int(target.get("station_id", -1)) >= 0:
		return "station"
	if int(target.get("body_id", -1)) >= 0:
		return "body"
	return ""


func _world_target_exists(target: Dictionary) -> bool:
	match _target_kind(target):
		"ship":
			return _ships.has(int(target.ship_id))
		"gate":
			for entry: Variant in _gates:
				if (entry as GateRecord).gate_id == int(target.gate_id):
					return true
		"station":
			for entry: Variant in _stations:
				if (entry as StationRecord).station_id == int(target.station_id):
					return true
		"body":
			for entry: Variant in _bodies:
				if (entry as CelestialBodyRecord).body_id == int(target.body_id):
					return true
	return false


func _target_distance_m(target: Dictionary) -> float:
	if _player_ship_id < 0 or not _ships.has(_player_ship_id):
		return -1.0
	var ship_position := _world.to_server_components(
		(_ships[_player_ship_id] as Node3D).global_position)
	match _target_kind(target):
		"ship":
			var target_id: int = int(target.ship_id)
			if _ships.has(target_id):
				var target_position := _world.to_server_components(
					(_ships[target_id] as Node3D).global_position)
				return _world.distance_components(ship_position, target_position)
		"gate":
			for entry: Variant in _gates:
				var gate := entry as GateRecord
				if gate.gate_id == int(target.gate_id):
					return _world.distance_components(ship_position, gate.position)
		"station":
			for entry: Variant in _stations:
				var station := entry as StationRecord
				if station.station_id == int(target.station_id):
					return _world.distance_components(ship_position, station.position)
		"body":
			for entry: Variant in _bodies:
				var body := entry as CelestialBodyRecord
				if body.body_id == int(target.body_id):
					return _world.distance_components(ship_position, body.position)
	return -1.0


func _radial_entries_for_target(target: Dictionary) -> Array[Dictionary]:
	var entries: Array[Dictionary] = []
	var kind := _target_kind(target)
	if kind.is_empty() or not _world_target_exists(target) or _player_ship_id < 0:
		return entries

	var target_gate_id: int = int(target.get("gate_id", -1))
	var target_station_id: int = int(target.get("station_id", -1))
	var nearby_gate_id: int = _nearby_gate_id if target_gate_id == _nearby_gate_id else -1
	var nearby_station_id: int = target_station_id if _nearby_station_ids.has(target_station_id) else -1
	var action_codes: Array[int] = _interaction.available_target_actions(
		_player_ship_id,
		int(target.get("ship_id", -1)),
		target_gate_id,
		int(target.get("body_id", -1)),
		target_station_id,
		nearby_gate_id,
		nearby_station_id,
		_target_distance_m(target))
	for action: int in action_codes:
		entries.append(_radial_entry(action, _radial_label_for_action(action)))
	return entries


func _radial_label_for_action(action: int) -> String:
	match action:
		WorldInteractionScript.TARGET_APPROACH:
			return "Approach"
		WorldInteractionScript.TARGET_WARP:
			return "Warp"
		WorldInteractionScript.TARGET_ORBIT:
			return "Orbit"
		WorldInteractionScript.TARGET_KEEP_AT_RANGE:
			return "Keep at range"
		WorldInteractionScript.TARGET_LOCK:
			return "Lock"
		WorldInteractionScript.TARGET_DOCK:
			return "Dock"
		WorldInteractionScript.TARGET_JUMP:
			return "Jump"
	return ""


func _radial_entry(action: int, label: String) -> Dictionary:
	return {
		"action": action,
		"label": label,
		"icon": _radial_icon_for_action(action),
		"angle": _radial_angle_for_action(action),
	}


func _radial_icon_for_action(action: int) -> String:
	match action:
		WorldInteractionScript.TARGET_APPROACH:
			return "approach"
		WorldInteractionScript.TARGET_WARP:
			return "warp"
		WorldInteractionScript.TARGET_ORBIT:
			return "orbit"
		WorldInteractionScript.TARGET_KEEP_AT_RANGE:
			return "range"
		WorldInteractionScript.TARGET_LOCK:
			return "lock"
		WorldInteractionScript.TARGET_DOCK:
			return "dock"
		WorldInteractionScript.TARGET_JUMP:
			return "jump"
	return ""


func _radial_angle_for_action(action: int) -> float:
	match action:
		WorldInteractionScript.TARGET_APPROACH:
			return 3.0 * PI / 4.0
		WorldInteractionScript.TARGET_WARP:
			return -3.0 * PI / 4.0
		WorldInteractionScript.TARGET_DOCK:
			return -PI / 2.0
		WorldInteractionScript.TARGET_JUMP:
			return -PI / 4.0
		WorldInteractionScript.TARGET_LOCK:
			return 0.0
		WorldInteractionScript.TARGET_KEEP_AT_RANGE:
			return PI / 4.0
		WorldInteractionScript.TARGET_ORBIT:
			return PI / 2.0
	return 0.0


func _target_title(target: Dictionary) -> String:
	match _target_kind(target):
		"ship":
			return "SHIP #%d" % int(target.ship_id)
		"gate":
			return "GATE #%d" % int(target.gate_id)
		"station":
			for entry: Variant in _stations:
				var station := entry as StationRecord
				if station.station_id == int(target.station_id):
					return station.name
		"body":
			for entry: Variant in _bodies:
				var body := entry as CelestialBodyRecord
				if body.body_id == int(target.body_id):
					return body.name
	return "Target"


func _update_radial_hold() -> void:
	if not _world_press_pending or _radial_open or not Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT):
		return
	if float(Time.get_ticks_msec()) / 1000.0 - _world_press_started_sec < WORLD_RADIAL_HOLD_SEC:
		return
	var hovered_control := get_viewport().gui_get_hovered_control()
	if hovered_control != null and hovered_control.mouse_filter != Control.MOUSE_FILTER_IGNORE:
		return
	if _camera.call("is_dragging") or _session.is_docked() or not _world_target_exists(_world_press_target):
		return
	var entries := _radial_entries_for_target(_world_press_target)
	if entries.is_empty():
		return
	_interaction.clear_click_sequence()
	_camera.call("end_orbit_drag")
	_radial_menu.call("open_menu", _world_press_position, _target_title(_world_press_target), entries)
	_radial_menu.call("update_pointer", get_viewport().get_mouse_position())
	_radial_open = true


func _finish_world_click() -> void:
	var camera_dragging: bool = _camera.call("is_dragging")
	var elapsed_sec := float(Time.get_ticks_msec()) / 1000.0 - _world_press_started_sec
	var target := _world_press_target
	_world_press_pending = false
	_world_press_target = {}
	_camera.call("end_orbit_drag")
	if camera_dragging or _session.is_docked():
		_interaction.clear_click_sequence()
		return
	if elapsed_sec >= WORLD_RADIAL_HOLD_SEC:
		_interaction.clear_click_sequence()
		return
	if not _world_target_exists(target) and _target_kind(target) != "":
		_interaction.clear_click_sequence()
		return
	var action: ClientAction = _interaction.interpret_primary_click(
		_world_press_position,
		float(Time.get_ticks_msec()) / 1000.0,
		false,
		_player_ship_id,
		int(target.get("ship_id", -1)),
		int(target.get("gate_id", -1)),
		int(target.get("body_id", -1)),
		int(target.get("station_id", -1)))
	_execute_client_action(action)


func _finish_radial_menu(screen_pos: Vector2) -> void:
	var action_code: int = _radial_menu.call("selected_action", screen_pos)
	var target := _world_press_target
	_radial_menu.call("close_menu")
	_radial_open = false
	_world_press_pending = false
	_world_press_target = {}
	_camera.call("end_orbit_drag")
	if action_code == 0 or not _world_target_exists(target) or _session.is_docked():
		_interaction.clear_click_sequence()
		return
	_update_gate_proximity()
	_update_station_proximity()
	var target_gate_id: int = int(target.get("gate_id", -1))
	var nearby_gate_id: int = _nearby_gate_id if target_gate_id == _nearby_gate_id else -1
	var target_station_id: int = int(target.get("station_id", -1))
	var nearby_station_id: int = target_station_id if _nearby_station_ids.has(target_station_id) else -1
	var action: ClientAction = _interaction.interpret_target_action(
		action_code,
		_player_ship_id,
		int(target.get("ship_id", -1)),
		int(target.get("gate_id", -1)),
		int(target.get("body_id", -1)),
		int(target.get("station_id", -1)),
		nearby_gate_id,
		nearby_station_id,
		_keep_at_range_km * 1000.0,
		_target_distance_m(target))
	if action_code == WorldInteractionScript.TARGET_LOCK:
		_try_lock_on(action)
	else:
		_execute_client_action(action)
	_interaction.clear_click_sequence()


func _cancel_radial_menu() -> void:
	_radial_menu.call("close_menu")
	_radial_open = false
	_world_press_pending = false
	_world_press_target = {}
	_camera.call("end_orbit_drag")
	_interaction.clear_click_sequence()

## Server-unit distance from the player ship to the selected gate, or -1 if
## there is no player ship or no selected gate (ADR-0022 warp gating / HUD).
func _selected_gate_distance() -> float:
	var selected_gate_id: int = _interaction.selected_gate_id()
	if selected_gate_id < 0 or _player_ship_id < 0 or not _ships.has(_player_ship_id):
		return -1.0
	var ship_server := _world.to_server_components((_ships[_player_ship_id] as Node3D).global_position)
	for entry: Variant in _gates:
		var gate: GateRecord = entry as GateRecord
		if gate.gate_id != selected_gate_id:
			continue
		return _world.distance_components(ship_server, gate.position)
	return -1.0

# -- Right-click -> LockOnCommand ---------------------------------------------


func _try_lock_on(action: ClientAction) -> void:
	var target_ship_id: int = action.target_ship_id()
	if _player_ship_id < 0 or target_ship_id < 0:
		return
	## Clear previous lock target
	if _player_lock_target >= 0 and _ships.has(_player_lock_target):
		(_ships[_player_lock_target] as Node3D).call("set_lock_state", "none")
	_player_lock_target = target_ship_id
	_connection.send_action(action)
	## Set Locking state and flash indicator
	if _ships.has(target_ship_id):
		(_ships[target_ship_id] as Node3D).call("set_lock_state", "locking")
		(_ships[target_ship_id] as Node3D).call("flash_lock_indicator")

# -- Double-click -> MoveCommand ----------------------------------------------

func _on_double_click(screen_pos: Vector2) -> void:
	if _player_ship_id < 0:
		return

	## Camera ray direction used directly as thrust direction (3D)
	var ray_dir: Vector3 = _camera.project_ray_normal(screen_pos)

	## Camera ray direction in server space (the server only uses its bearing:
	## it normalizes target - ship, so the scale factor is irrelevant).
	var server_dir: Vector3 = _world.dir_to_server(ray_dir)

	## Estimate player ship position in server space (back-calculated from lerped Godot position)
	var ship_server_pos: Vector3 = Vector3.ZERO
	if _ships.has(_player_ship_id):
		ship_server_pos = _world.to_server((_ships[_player_ship_id] as Node3D).global_position)

	## Set target far away so server treats normalize(target - ship) as server_dir
	var target: Vector3 = ship_server_pos + server_dir * 1_000_000.0
	_connection.send_move_command(target)

	## Show thrust arrow on player ship (ray_dir stays in Godot space)
	if _ships.has(_player_ship_id):
		(_ships[_player_ship_id] as Node3D).call("set_thrust_direction", ray_dir)

# -- S key -> StopCommand -----------------------------------------------------

func _apply_stop_prediction() -> void:
	if _player_ship_id < 0:
		return
	## Apply braking immediately in the local predictor, then clear the arrow.
	if _ships.has(_player_ship_id):
		(_ships[_player_ship_id] as Node3D).call("set_braking")

# -- Event handlers -----------------------------------------------------------

func _handle_position_snap(ship_id: int, server_pos: PackedFloat64Array) -> void:
	if not _ships.has(ship_id):
		return
	if ship_id == _player_ship_id:
		var pg: Vector3 = _ship_position(_ships[ship_id] as Node3D)
		var new_origin := PackedFloat64Array([
			server_pos[0] - pg.x / _world.render_scale(),
			server_pos[1] - pg.y / _world.render_scale(),
			server_pos[2] + pg.z / _world.render_scale()])
		_presentation.apply_origin_rebase_components(new_origin, true, _ships)
	else:
		(_ships[ship_id] as Node3D).call(
			"reset_motion", server_pos, Vector3.ZERO, _session.current_tick())
	if ship_id == _player_ship_id:
		(_ships[ship_id] as Node3D).call(
			"reset_motion", server_pos, Vector3.ZERO, _session.current_tick())
func _handle_ship_docked(
	ship_id: int,
	station_id: int,
	tick: int,
	session_accepted: bool
) -> void:
	if not _ships.has(ship_id):
		return
	if ship_id == _player_ship_id and not session_accepted:
		return
	var ship := _ships[ship_id] as Node3D
	var dock_pos: PackedFloat64Array = ship.call("server_position") as PackedFloat64Array
	for entry: Variant in _stations:
		var station: StationRecord = entry as StationRecord
		if station.station_id == station_id:
			dock_pos = station.position
			break
	if not (ship.call("dock_motion", dock_pos, tick) as bool):
		return
	_sync_session_state()
func _handle_ship_undocked(
	ship_id: int,
	station_id: int,
	tick: int,
	session_accepted: bool
) -> void:
	if ship_id == _player_ship_id and not session_accepted:
		return
	if _ships.has(ship_id):
		var ship := _ships[ship_id] as Node3D
		if not (ship.call(
			"undock_motion",
			ship.call("server_position"),
			Vector3.ZERO,
			tick) as bool):
			return
	if ship_id == _player_ship_id:
		_nearby_station_ids.clear()
		if station_id >= 0:
			_nearby_station_ids.append(station_id)
	_sync_session_state()
func _ship_position(ship: Node3D) -> Vector3:
	return ship.global_position if ship.is_inside_tree() else ship.position

# -- Jump Gate (ADR-0009) -----------------------------------------------------

## Ship passed through a Jump Gate -- teleport to entry_pos.
func _handle_jump_gate_used(
	ship_id: int,
	gate_id: int,
	entry_pos: PackedFloat64Array,
	tick: int
) -> void:
	if not _ships.has(ship_id):
		return
	(_ships[ship_id] as Node3D).call("update_target", entry_pos, tick)
	if ship_id == _player_ship_id:
		_jump_notice = "Jumped via Gate #%d" % gate_id
		_jump_notice_timer = 3.0
func _handle_star_system_changed(
	_ship_id: int,
	_to_system: int,
	to_name: Variant
) -> void:
	_sync_session_state()
	if to_name != null:
		_jump_notice = "Entered %s system" % to_name
		_jump_notice_timer = 3.0
		_interaction.clear_navigation_selection()
func _on_connection_changed(connected: bool) -> void:
	if not connected:
		_clear_all_ships()

func _on_market_refresh() -> void:
	if not _session.is_docked():
		return
	_connection.send_market_refresh_command()


func _on_market_place_order(
	item_id: ItemIdentity,
	side: String,
	price: int,
	quantity: int
) -> void:
	if _player_ship_id < 0 or not _session.is_docked() or item_id == null:
		return
	_connection.send_market_place_order_command(
		_player_ship_id, item_id, side, price, quantity)


func _on_market_cancel_order(order_id: int) -> void:
	if not _session.is_docked():
		return
	_connection.send_market_cancel_order_command(order_id)


func _on_market_snapshot(snapshot: MarketSnapshot) -> void:
	_market_surface.apply_snapshot(snapshot)
func _on_initial_state(state: InitialStatePresentation) -> void:
	_clear_ship_nodes()
	_hud_surface.hide_duel_result()
	_ingest_star_map()
	for ship_entry: Variant in state.ships:
		_spawn_ship_from_record(
			ship_entry as ShipPresentation,
			(ship_entry as ShipPresentation).ship_id == _session.player_ship_id())
func _ingest_star_map() -> void:
	_sync_session_state()
	_gates = _session.gates()
	_stations = _session.stations()
	_bodies = _session.bodies()
	_buildable_ship_types = _session.buildable_ship_types()
	_presentation.respawn_navigation_markers(
		_gates,
		_bodies,
		_stations,
		Callable(_interaction, "clear_navigation_selection")
	)
func _instantiate_ship(sid: int, server_pos: PackedFloat64Array) -> Node3D:
	var ship: Node3D = SHIP_SCENE.instantiate() as Node3D
	_ships_root.add_child(ship)
	ship.call("initialize", sid, server_pos, _world.origin_components())
	ship.name = "Ship_%d" % sid
	return ship

## Materialize one ship node from a typed presentation record. Shared by
## InitialState and AoiEnter (ADR-0019). Skips ships already present.
func _spawn_ship_from_record(record: ShipPresentation, became_player: bool) -> void:
	var sid: int = record.ship_id
	if _ships.has(sid):
		return
	var server_pos: PackedFloat64Array = record.position
	var ship: Node3D = _instantiate_ship(sid, server_pos)
	ship.set_meta("callout_text", "%s #%d" % [record.ship_type_name, sid])
	ship.call(
		"configure_motion",
		record.max_speed,
		record.mass,
		record.inertia_modifier,
		server_pos,
		Vector3(record.velocity[0], record.velocity[1], record.velocity[2]),
		_session.current_tick())
	_ships[sid] = ship
	_sync_session_state()
	if became_player:
		_set_as_player_ship(sid, ship)
func _handle_aoi_enter(
	ship: ShipPresentation,
	registered: bool,
	became_player: bool
) -> void:
	if registered:
		_spawn_ship_from_record(ship, became_player)
func _handle_aoi_leave(sid: int, removed: bool) -> void:
	var ship: Node3D = _ships.get(sid) as Node3D
	_sync_session_state()
	if not removed:
		return
	_interaction.clear_target_if_matches(sid)
	_ships.erase(sid)
	if ship != null:
		ship.queue_free()
func _on_player_fitting() -> void:
	_apply_loadout_side_effects()
func _apply_loadout_side_effects() -> void:
	var new_active_ship_id: int = _session.player_ship_id()
	var attached_new_player := false
	if new_active_ship_id != _player_ship_id:
		if new_active_ship_id >= 0 and _ships.has(new_active_ship_id):
			_set_as_player_ship(new_active_ship_id, _ships[new_active_ship_id] as Node3D)
			attached_new_player = true
		elif new_active_ship_id < 0:
			_player_ship_id = new_active_ship_id
			_presentation.detach_player_ship()
	_sync_session_state()
	if not _session.is_docked() and _market_surface.is_open():
		_market_surface.set_open(false)
	if not attached_new_player and _player_ship_id >= 0:
		_apply_current_dock_state_to_player_ship(
			_ships.get(_player_ship_id) as Node3D)
	var modules := _loadout.modules()
	var inventory := _loadout.inventory()
	var station_inventory := _loadout.station_inventory()
	var owned_ships := _loadout.owned_ships()
	_hud_surface.set_player_fitting(
		modules, inventory, station_inventory, owned_ships, _buildable_ship_types)
	_market_surface.set_cargo(inventory)
	_recalc_weapon_range()
func _recalc_weapon_range() -> void:
	_weapon_range = _loadout.weapon_optimal_range()
	_weapon_falloff = _loadout.weapon_falloff_range()
	_presentation.update_tactical_overlay_ranges(_weapon_range, _weapon_falloff)

func _on_module_activated(p_ship_id: int, _p_module_id: int, _slot: String) -> void:
	if p_ship_id == _player_ship_id:
		_recalc_weapon_range()
func _on_module_deactivated(
	p_ship_id: int,
	_p_module_id: int,
	_slot: String,
	_reason: String
) -> void:
	if p_ship_id == _player_ship_id:
		_recalc_weapon_range()
func _apply_player_module_activation(module_id: int, active: bool, forced_reason: String) -> void:
	_loadout.apply_module_activation(module_id, active, forced_reason)
	_recalc_weapon_range()


## Inventory row behavior is resolved by the engine-independent station policy.
## This method only supplies current context and applies the resulting request
## or local Build picker effect.
func _handle_inventory_row_click(row: InventoryRow) -> void:
	if row == null or row.policy_row == null:
		return
	var action: StationInventoryAction = _station_inventory.click(
		row.policy_row, _player_ship_id, _session.docked_station_id(), _loadout.modules())
	_execute_station_inventory_action(action)


func _execute_station_inventory_action(action: StationInventoryAction) -> void:
	if action == null:
		return
	if action.is_build_picker_toggle():
		_hud_surface.toggle_build_picker(
			_loadout.modules(),
			_loadout.inventory(),
			_loadout.station_inventory(),
			_loadout.owned_ships(),
			_buildable_ship_types)
	elif action.request_count() > 0:
		_connection.send_station_inventory_action(action)


## Right-click on a SHIP CARGO row moves the whole stack to the docked
## station's inventory (ADR-0034 9B). Uniform across item types (Module,
## ScrapMetal) per the user's explicit preference for a single straightforward
## right-click gesture rather than per-type UI carve-outs.
func _handle_inventory_row_right_click(row: InventoryRow) -> void:
	if row == null or row.policy_row == null:
		return
	var action: StationInventoryAction = _station_inventory.resolve_drop(
		row.policy_row,
		StationInventoryInteraction.column_station(),
		StationInventoryRow.none(),
		_player_ship_id,
		_session.docked_station_id())
	_execute_station_inventory_action(action)


## Lazily creates the drag ghost once the cursor has moved past
## DRAG_THRESHOLD_PX (avoids a ghost flash on an ordinary click) and keeps it
## following the cursor.
func _update_drag_ghost(pos: Vector2) -> void:
	if _drag_row == null:
		return
	if _drag_ghost == null:
		if pos.distance_to(_drag_start_pos) < DRAG_THRESHOLD_PX:
			return
		var label: Label = (_drag_row.panel as Panel).get_child(0) as Label
		_drag_ghost = _hud_surface.create_drag_ghost(label.text)
	_drag_ghost.position = pos + Vector2(12.0, 12.0)


func _clear_drag_ghost() -> void:
	if _drag_ghost != null:
		_drag_ghost.queue_free()
		_drag_ghost = null


## Resolves a mouse-up while `_drag_row` is set: a plain click (moved less
## than DRAG_THRESHOLD_PX since press) fires the existing click handler
## exactly as before this feature existed; past the threshold, it's a drop,
## dispatched by (origin column, target column) below.
func _end_inventory_drag(release_pos: Vector2) -> void:
	var row: InventoryRow = _drag_row
	var start: Vector2 = _drag_start_pos
	_drag_row = null
	_clear_drag_ghost()
	if row == null:
		return
	if release_pos.distance_to(start) < DRAG_THRESHOLD_PX:
		_handle_inventory_row_click(row)
		return
	var target_column: int = _hud_surface.inventory_panel_column_at(release_pos)
	_handle_inventory_row_drop(row, target_column, release_pos)


## Dispatch matrix for a drag that ended in `target_column`. Right-click
## transfer and plain-click fit/unfit/etc. are untouched by this -- drag is
## an additive path to the same commands, same "keep both interaction paths"
## precedent as the Build/Disassemble buttons-plus-keys work.
func _handle_inventory_row_drop(row: InventoryRow, target_column: int, release_pos: Vector2) -> void:
	if row == null or row.policy_row == null \
			or target_column == StationInventoryInteraction.column_none():
		return
	var target_policy_row := StationInventoryRow.none()
	if target_column == StationInventoryInteraction.column_fitted():
		var target_row: InventoryRow = _hud_surface.inventory_panel_row_at(release_pos)
		if target_row != null and target_row.policy_row != null:
			target_policy_row = target_row.policy_row
	var action: StationInventoryAction = _station_inventory.resolve_drop(
		row.policy_row,
		target_column,
		target_policy_row,
		_player_ship_id,
		_session.docked_station_id())
	_execute_station_inventory_action(action)


func _toggle_module_by_index(f_index: int) -> void:
	## F1-F8 map to active module indices 0-7 (High/Mid slots).
	## Godot supplies current world facts; ClientLoadout owns the decision.
	var has_target_distance := false
	var target_distance_units := -1.0
	if _player_lock_target >= 0 and _player_ship_id >= 0:
		if _ships.has(_player_lock_target) and _ships.has(_player_ship_id):
			target_distance_units = _distance_in_server_units(
				(_ships[_player_ship_id] as Node3D).global_position,
				(_ships[_player_lock_target] as Node3D).global_position,
				_world.render_scale())
	if target_distance_units >= 0.0:
		has_target_distance = true
	var decision: ModuleActivationDecision = _loadout.resolve_activation(
		f_index, _player_ship_id >= 0, _player_lock_target,
		has_target_distance, target_distance_units)
	var mid: int = decision.module_id() as int
	var slot: String = decision.slot()
	match decision.action():
		"noop":
			return
		"rejected":
			match decision.rejection_reason():
				"no_locked_target":
					_jump_notice = "No target locked"
				"target_unavailable", "out_of_range":
					_jump_notice = "Target out of range"
			_jump_notice_timer = 2.0
			return
		"deactivate":
			_apply_player_module_activation(mid, false, "")
			_connection.send_deactivate_module(mid, slot)
		"activate":
			_apply_player_module_activation(mid, true, "")
			_connection.send_activate_module(mid, slot, decision.target_ship_id())
		_:
			return

static func _distance_in_server_units(
	from_position: Vector3, to_position: Vector3, render_scale: float
) -> float:
	if not is_finite(render_scale) or render_scale <= 0.0:
		return -1.0
	return from_position.distance_to(to_position) / render_scale

func _set_as_player_ship(p_ship_id: int, ship: Node3D) -> void:
	_player_ship_id = p_ship_id
	_presentation.attach_player_ship(ship, _weapon_range, _weapon_falloff)
	_apply_current_dock_state_to_player_ship(ship)


func _apply_current_dock_state_to_player_ship(ship: Node3D) -> void:
	if ship == null or not _session.is_docked():
		return
	ship.call(
		"dock_motion",
		ship.call("server_position"),
		_session.latest_dock_state_tick())

# -- Domain event handlers ----------------------------------------------------

func _handle_ship_spawned(
	ship_id: int,
	position: PackedFloat64Array,
	registered: bool,
	became_player: bool
) -> void:
	if not registered or _ships.has(ship_id):
		return
	var ship: Node3D = _instantiate_ship(ship_id, position)
	_ships[ship_id] = ship
	_sync_session_state()
	if became_player:
		_set_as_player_ship(ship_id, ship)
func _handle_velocity_changed(
	ship_id: int,
	velocity: PackedFloat64Array,
	tick: int
) -> void:
	if not _ships.has(ship_id):
		return
	(_ships[ship_id] as Node3D).call(
		"set_velocity", Vector3(velocity[0], velocity[1], velocity[2]), tick)
	_sync_session_state()
func _handle_motion_correction(correction: MotionCorrectionPresentation) -> void:
	if correction.ship_id != _player_ship_id or not _ships.has(correction.ship_id):
		return
	(_ships[correction.ship_id] as Node3D).call(
		"reconcile_motion",
		correction.position,
		Vector3(correction.velocity[0], correction.velocity[1], correction.velocity[2]),
		correction.tick)
func _handle_ship_despawned(ship_id: int, removed: bool) -> void:
	var ship: Node3D = _ships.get(ship_id) as Node3D
	_sync_session_state()
	if not removed:
		return
	_ships.erase(ship_id)
	if ship != null:
		ship.queue_free()
	_interaction.clear_target_if_matches(ship_id)
func _handle_damage_taken(ship_id: int) -> void:
	_sync_session_state()
	if _ships.has(ship_id):
		(_ships[ship_id] as Node3D).call("flash_damage")
func _handle_repair_applied(ship_id: int) -> void:
	_sync_session_state()
	if _ships.has(ship_id):
		(_ships[ship_id] as Node3D).call("flash_repair")
func _handle_ship_destroyed(ship_id: int, outcome: DestructionOutcome) -> void:
	var ship: Node3D = _ships.get(ship_id) as Node3D
	_sync_session_state()
	if not outcome.destroyed:
		return
	_ships.erase(ship_id)
	if ship == null:
		return
	_interaction.clear_target_if_matches(ship_id)
	ship.call("play_destroy_effect")
	if outcome.destroyed_player:
		_hud_surface.show_duel_result(false)
	elif outcome.destroyed_opponent:
		_hud_surface.show_duel_result(true)
func _handle_target_locked(_locker_id: int, target_id: int, changed: bool) -> void:
	_sync_session_state()
	if changed and _ships.has(target_id):
		(_ships[target_id] as Node3D).call("set_lock_state", "locked")
func _handle_lock_lost(_locker_id: int, target_id: int, changed: bool) -> void:
	_sync_session_state()
	if changed and _ships.has(target_id):
		(_ships[target_id] as Node3D).call("set_lock_state", "none")
func _update_hud() -> void:
	var facts := HudSceneFacts.new()
	facts.connected = _connection.is_connected_to_server()
	if _player_ship_id >= 0 and _ships.has(_player_ship_id):
		facts.has_player_speed = true
		facts.player_speed_units = (_ships[_player_ship_id] as Node3D).call("get_speed_server") as float
	facts.target_known = _ships.has(_player_lock_target)
	if _ships.has(_player_lock_target) and _player_ship_id >= 0 and _ships.has(_player_ship_id):
		var target_distance := _distance_in_server_units(
			(_ships[_player_ship_id] as Node3D).global_position,
			(_ships[_player_lock_target] as Node3D).global_position,
			_world.render_scale())
		if target_distance >= 0.0:
			facts.has_target_distance = true
			facts.target_distance_units = target_distance
	facts.nearby_gate_id = _nearby_gate_id
	facts.nearby_station_ids = _nearby_station_ids
	facts.jump_notice = _jump_notice
	facts.selected_gate_id = _interaction.selected_gate_id()
	facts.selected_body_id = _interaction.selected_body_id()
	facts.selected_station_id = _interaction.selected_station_id()
	facts.selected_target_id = _interaction.selected_target_id()
	var gate_distance := _selected_gate_distance()
	if gate_distance >= 0.0:
		facts.has_selected_gate_distance = true
		facts.selected_gate_distance_units = gate_distance
	facts.keep_at_range_km = _keep_at_range_km
	_hud_surface.paint(_hud_read_model.project(_session, _loadout, facts))

# -- Capacitor client-side simulation -----------------------------------------

## Mirror of CapacitorSystem::run() -- called once per tick elapsed.
## Keeps cap display in sync without any extra server messages.
func _simulate_cap(ticks: int) -> void:
	_session.advance_client_ticks(ticks, _loadout)
	_sync_session_state()

func _advance_client_cap_ticks(delta: float) -> void:
	var cap_current: float = _session.capacitor_status().current
	if _player_ship_id < 0 or cap_current < 0.0:
		_cap_tick_accumulator = 0.0
		return
	_cap_tick_accumulator += delta * CLIENT_TICKS_PER_SEC
	var ticks: int = int(floor(_cap_tick_accumulator))
	if ticks <= 0:
		return
	_cap_tick_accumulator -= float(ticks)
	_simulate_cap(ticks)

# -- Internal utilities -------------------------------------------------------

func _clear_ship_nodes() -> void:
	for ship_node: Node3D in _ships.values():
		if is_instance_valid(ship_node):
			ship_node.queue_free()
	_ships.clear()


func _clear_all_ships() -> void:
	_clear_ship_nodes()
	_session.reset()
	_hud_read_model.reset()
	_sync_session_state()
	## _session.reset() clears WorldSession's navigation map too -- clear the
	## caches to match, so a disconnect doesn't leave stale gate/station/body
	## data sitting until the next InitialState's _ingest_star_map() call.
	_gates = []
	_stations = []
	_bodies = []
	_buildable_ship_types = []
	_interaction.clear_selection()
	_loadout.reset()
	_cap_tick_accumulator = 0.0


## Reconciles the two fields main.gd writes optimistically ahead of the
## server's confirming event (_player_ship_id, _player_lock_target) against
## WorldSession's authoritative state. Fast-changing fields (ship health,
## capacitor status, current system name) are read directly at point of use
## (_session.ship_health(id)/.capacitor_status()/.current_system_name())
## instead of being mirrored here; the write-once navigation map
## (gates/stations/bodies/buildable ship types) is cached separately in
## _ingest_star_map() (ADR-0046).
func _sync_session_state() -> void:
	_player_ship_id = _session.player_ship_id() as int
	_player_lock_target = _session.player_lock_target() as int
