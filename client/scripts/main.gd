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
@onready var _warp_tunnel  : ColorRect   = $HUD/WarpTunnel

# -- Constants ----------------------------------------------------------------

const SHIP_SCENE  := preload("res://scenes/ship.tscn")
const HudSurfaceScript = preload("res://scripts/hud_surface.gd")
const MarketSurfaceScript = preload("res://scripts/market_surface.gd")
const WorldPresentationScript = preload("res://scripts/world_presentation.gd")
const PositionComponents = preload("res://scripts/position_components.gd")
const WorldInteractionScript = preload("res://scripts/world_interaction.gd")
const InventoryRow = preload("res://scripts/inventory_row.gd")
## Unit-to-metre scale: real metres = (units/tick or units) * METERS_PER_UNIT,
## fed into UnitFormat for display (m/s, km/s, AU/s, ... -- whichever reads
## best at the given magnitude). Change this one constant to rescale all
## displayed speeds and distances.
const METERS_PER_UNIT : float = 1.0
const CLIENT_TICKS_PER_SEC : float = 10.0

const BUILDABLE_SHIP_TYPE_ID : int = 7

var _cap_tick_accumulator : float = 0.0

# -- Internal state -----------------------------------------------------------

var _session := WorldSession.new()
var _interaction := WorldInteractionScript.new()
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
	if _warp_tunnel  == null: missing.append("Warp tunnel ($HUD/WarpTunnel)")
	if not missing.is_empty():
		push_error("main.tscn is missing expected node(s): %s. Check the scene tree against the @onready paths in main.gd." % ", ".join(missing))

func _ready() -> void:
	_assert_scene_tree_refs()
	_connection.bind_client_state(_session, _loadout)
	_connection.event_received.connect(_on_event_received)
	_connection.motion_correction_received.connect(_handle_motion_correction)
	_connection.connection_changed.connect(_on_connection_changed)
	_connection.welcomed.connect(_on_welcomed)
	_connection.initial_state_received.connect(_on_initial_state)
	_connection.player_fitting_received.connect(_on_player_fitting)
	_connection.module_activated.connect(_on_module_activated)
	_connection.module_deactivated.connect(_on_module_deactivated)
	_connection.market_snapshot_received.connect(_on_market_snapshot)
	_presentation.build(self, _camera, _warp_tunnel, _gates_root, _bodies_root, _world)
	_hud_surface.build(self, _hud, _stats_label)
	_market_surface.build(
		_hud,
		Callable(self, "_on_market_refresh"),
		Callable(self, "_on_market_place_order"),
		Callable(self, "_on_market_cancel_order"))
	_update_hud()
	## Gate / body markers are spawned from the server's InitialState, not here.

func _process(delta: float) -> void:
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
var _client_rules := ClientRules.new()

## Real-unit (m/s, km/s, AU/s, ...) display formatting (ADR-0029 §1.5: single
## conversion module). Static methods only -- preloaded rather than referenced
## by global class_name for the same headless-test-cache reason as WorldSpace.
const UnitFormat = preload("res://scripts/unit_format.gd")

## Converts a legacy f32 server-space position (Y-up, +Z) into Godot world
## space (Y-up, -Z), relative to the floating origin. Absolute wire positions
## use `_server_components_to_godot` so they are narrowed only after subtraction.
## Thin wrapper kept for legacy event payloads and tests.
func _server_to_godot_pos(p: Vector3) -> Vector3:
	return _world.to_godot(p)

## Reads a legacy f32 {x,y,z} sub-dictionary (for example PosWire event fields)
## into a Vector3. Absolute f64 position fields use the component helper below.
func _vec3_from_dict(d: Dictionary, key: String) -> Vector3:
	var value: Dictionary = d.get(key, {}) as Dictionary
	return Vector3(
		value.get("x", 0.0) as float,
		value.get("y", 0.0) as float,
		value.get("z", 0.0) as float)

func _position_components_from_dict(d: Dictionary, key: String) -> PackedFloat64Array:
	return PositionComponents.from_dict(d, key)

func _position_components(value: Variant) -> PackedFloat64Array:
	return PositionComponents.from_value(value)

func _server_components_to_godot(position: PackedFloat64Array) -> Vector3:
	return _world.to_godot_components(position[0], position[1], position[2])

func _velocity_components_to_vec3(velocity: PackedFloat64Array) -> Vector3:
	return Vector3(velocity[0], velocity[1], velocity[2])

## Reads a {dx,dy,dz} velocity sub-dictionary from a wire payload.
func _velocity_from_dict(d: Dictionary, key: String = "velocity") -> Vector3:
	var velocity: Dictionary = d.get(key, {}) as Dictionary
	return Vector3(
		velocity.get("dx", 0.0) as float,
		velocity.get("dy", 0.0) as float,
		velocity.get("dz", 0.0) as float)

## Tracks whether the player ship is within activation range of a Jump Gate
## (ADR-0009). Distance is computed in server units by WorldSpace.
func _update_gate_proximity() -> void:
	_nearby_gate_id = -1
	if _player_ship_id < 0 or not _ships.has(_player_ship_id):
		return
	var ship_pos := _world.to_server_components((_ships[_player_ship_id] as Node3D).global_position)
	for entry: Variant in _gates:
		var gate: GateRecord = entry as GateRecord
		var gate_pos := _position_components(gate.position)
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
		var station_pos := _position_components(station.position)
		var dist: float = _world.distance_components(ship_pos, station_pos)
		if dist <= station.docking_radius:
			in_range.append({"station_id": station.station_id, "distance": dist})
	in_range.sort_custom(func(a: Dictionary, b: Dictionary) -> bool:
		return (a.distance as float) < (b.distance as float))
	for entry: Dictionary in in_range:
		_nearby_station_ids.append(entry.station_id as int)


## Display name for a station_id, falling back to "Station #N" if unnamed
## or not found in the galaxy map (e.g. between InitialState and StarMap sync).
func _station_name(station_id: int) -> String:
	for entry: Variant in _stations:
		var station: StationRecord = entry as StationRecord
		if station.station_id == station_id:
			var name: String = station.name
			return name if not name.is_empty() else "Station #%d" % station_id
	return "Station #%d" % station_id

func _input(event: InputEvent) -> void:
	## Keyboard shortcuts: InputDecoder decides what the keypress means
	## (architecture-review/client.md C-1); this just performs the side
	## effects (network sends, warp-snap-pos / overlay state writes).
	if event is InputEventKey and event.pressed and not event.echo:
		if _market_surface.keyboard_consumes():
			return
		var key: InputEventKey = event as InputEventKey
		var nearest_station_id: int = _nearby_station_ids[0] if not _nearby_station_ids.is_empty() else -1
		var action: Dictionary = _interaction.resolve_key_action(
			key.keycode,
			_player_ship_id,
			_nearby_gate_id,
			nearest_station_id,
			_session.docked_station_id())
		match action.get("kind", "none") as String:
			"toggle_module":
				_toggle_module_by_index(action.module_index as int)
			"stop":
				_send_stop_command()
			"jump":
				var jump_gate: int = action.gate_id as int
				_connection.send_jump_command(jump_gate)
			"approach_gate":
				_connection.send_approach_gate_command(action.gate_id as int)
			"approach_ship":
				_connection.send_approach_command(action.ship_id as int)
			"warp_to_gate":
				_connection.send_warp_command(action.gate_id as int)
			"warp_to_body":
				_connection.send_warp_to_body_command(action.body_id as int)
			"orbit_gate":
				_connection.send_orbit_gate_command(action.gate_id as int)
			"orbit_ship":
				_connection.send_orbit_command(action.ship_id as int)
			"keep_at_range_gate":
				_connection.send_keep_at_range_gate_command(
					action.gate_id as int, _keep_at_range_km * 1000.0)
			"keep_at_range_ship":
				_connection.send_keep_at_range_command(
					action.ship_id as int, _keep_at_range_km * 1000.0)
			"adjust_keep_at_range":
				_keep_at_range_km = clampf(
					_keep_at_range_km + (action.delta_km as float),
					KEEP_AT_RANGE_MIN_KM, KEEP_AT_RANGE_MAX_KM)
				_update_hud()
			"toggle_tactical_overlay":
				_presentation.toggle_tactical_overlay()
			"toggle_inventory_panel":
				_hud_surface.toggle_inventory_panel()
			"toggle_market_panel":
				if _market_surface.toggle():
					_connection.send_market_refresh_command()
			"dock":
				_connection.send_dock_command(action.station_id as int)
			"undock":
				_connection.send_undock_command()
			"build_packaged_ship":
				_connection.send_build_packaged_ship_command(
					_player_ship_id,
					action.station_id as int,
					BUILDABLE_SHIP_TYPE_ID)
			"disassemble_ship":
				_connection.send_disassemble_ship_command(_player_ship_id, action.station_id as int)
			"disembark":
				_connection.send_disembark_command()
		return

	if event is InputEventMouseMotion and _drag_row != null:
		_update_drag_ghost((event as InputEventMouseMotion).position)
		return

	if event is InputEventMouseButton:
		var mb: InputEventMouseButton = event as InputEventMouseButton
		if mb.button_index == MOUSE_BUTTON_LEFT and not mb.pressed:
			if _drag_row != null:
				_end_inventory_drag(mb.position)
				return
			_camera.call("end_orbit_drag")
			return
		if mb.pressed:
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
					var hit_ship: int = _pick_ship_at(mb.position)
					var hit_gate: int = -1
					var hit_body: int = -1
					if hit_ship < 0:
						hit_gate = _pick_gate_at(mb.position)
						if hit_gate < 0:
							hit_body = _pick_body_at(mb.position)
					var click_action: Dictionary = _interaction.interpret_primary_click(
						mb.position,
						Time.get_ticks_msec() / 1000.0,
						(_camera as Node).call("is_dragging") as bool,
						_player_ship_id,
						hit_ship,
						hit_gate,
						hit_body)
					match click_action.get("kind", "none") as String:
						"double_click_move":
							_on_double_click(mb.position)
						"selection_changed":
							_update_hud()
				MOUSE_BUTTON_RIGHT:
					var lock_action: Dictionary = _interaction.interpret_lock_click(
						_player_ship_id,
						_pick_ship_at(mb.position))
					if (lock_action.get("kind", "none") as String) == "lock_on":
						_try_lock_on(lock_action.ship_id as int)

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
		return _world.distance_components(ship_server, _position_components(gate.position))
	return -1.0

# -- Right-click -> LockOnCommand ---------------------------------------------

func _try_lock_on(target_ship_id: int) -> void:
	if _player_ship_id < 0 or target_ship_id < 0:
		return
	## Clear previous lock target
	if _player_lock_target >= 0 and _ships.has(_player_lock_target):
		(_ships[_player_lock_target] as Node3D).call("set_lock_state", "none")
	_player_lock_target = target_ship_id
	_connection.send_lock_on_command(target_ship_id)
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

func _send_stop_command() -> void:
	if _player_ship_id < 0:
		return
	_connection.send_stop_command()
	## Apply braking immediately in the local predictor, then clear the arrow.
	if _ships.has(_player_ship_id):
		(_ships[_player_ship_id] as Node3D).call("set_braking")

# -- Event handlers -----------------------------------------------------------

func _on_event_received(outcome: ServerEventOutcome) -> void:
	_sync_session_state()
	if not outcome.dispatch(self):
		push_warning("[World] failed to dispatch typed server event outcome")
func _handle_position_snap(ship_id: int, server_pos: PackedFloat64Array) -> void:
	if not _ships.has(ship_id):
		return
	if ship_id == _player_ship_id:
		var pg: Vector3 = _ship_position(_ships[ship_id] as Node3D)
		var new_origin := PackedFloat64Array([
			server_pos[0] - pg.x / _world.render_scale(),
			server_pos[1] - pg.y / _world.render_scale(),
			server_pos[2] + pg.z / _world.render_scale()])
		_presentation.apply_origin_rebase_components(new_origin, true, _player_ship_id, _ships)
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
			dock_pos = _position_components(station.position)
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

## Welcome received: just record player_id / ship_id.
## Ship nodes are spawned by the subsequent InitialState.
func _on_welcomed(_p_player_id: int, _p_ship_id: int) -> void:
	## connection.gd ship_id / player_id properties are already populated.
	## Market access is station-gated, so do not query it while the player is
	## still in open space after connecting.
	return


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
		_server_components_to_godot,
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
	ship.call(
		"configure_motion",
		record.max_speed,
		record.mass,
		record.inertia_modifier,
		server_pos,
		_velocity_components_to_vec3(record.velocity),
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
	if new_active_ship_id != _player_ship_id:
		if new_active_ship_id >= 0 and _ships.has(new_active_ship_id):
			_set_as_player_ship(new_active_ship_id, _ships[new_active_ship_id] as Node3D)
		elif new_active_ship_id < 0:
			_player_ship_id = new_active_ship_id
			_presentation.detach_player_ship()
	_sync_session_state()
	if not _session.is_docked() and _market_surface.is_open():
		_market_surface.set_open(false)
	if _session.is_docked() and _player_ship_id >= 0:
		var docked_ship := _ships.get(_player_ship_id) as Node3D
		if docked_ship != null:
			docked_ship.call("dock_motion", docked_ship.call("server_position"), _loadout.tick())
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


## A row click in the inventory panel: "fit" sends the module's own slot kind
## (the module's `def.slot` decides where it goes -- the player makes no slot
## choice), "unfit" removes that exact fitted instance (ADR-0032).
func _handle_inventory_row_click(row: InventoryRow) -> void:
	match row.action:
		InventoryRow.ACTION_FIT:
			## Refitting requires being docked. The module ID is extracted
			## from the canonical Item identity only at this command seam.
			if row.item_id != null and row.item_id.is_module() \
					and _player_ship_id >= 0 and _session.is_docked():
				_connection.send_fit_module_command(
					_player_ship_id, row.item_id.module_id() as int, row.slot)
		InventoryRow.ACTION_UNFIT:
			if _player_ship_id >= 0 and _session.is_docked():
				_connection.send_unfit_module_command(_player_ship_id, row.module_id, row.slot)
		InventoryRow.ACTION_UNFIT_ALL:
			## No new wire command -- sends one UnfitModuleCommand per
			## currently-fitted module (non-atomic: a mid-loop failure leaves
			## a partially-unfitted ship, but each Unfit is independently safe
			## and this is a convenience action, not a transactional one).
			if _player_ship_id >= 0 and _session.is_docked():
				for entry: Variant in _loadout.modules():
					_connection.send_unfit_module_command(
						_player_ship_id, entry.module_id as int, entry.slot as String)
		InventoryRow.ACTION_ASSEMBLE:
			## No active-ship requirement: this is exactly the recovery path
			## for a shipless docked player (docs/architecture/ownership.md §8).
			var docked_station_id: int = _session.docked_station_id()
			if docked_station_id >= 0 and row.item_id != null \
					and row.item_id.is_packaged_ship():
				_connection.send_assemble_command(
					docked_station_id, row.item_id.ship_type_id() as int)
		InventoryRow.ACTION_SELECT_ACTIVE_SHIP:
			## Also no active-ship requirement -- this is how a player re-boards
			## after Disembark, or switches to a different owned ship.
			_connection.send_select_active_ship_command(row.ship_id)
		InventoryRow.ACTION_DISASSEMBLE:
			## Dedicated button alongside the existing [Y] key (Phase 9B task
			## 10) -- same command, server validates docked/undamaged/unfitted.
			if _player_ship_id >= 0:
				var docked_station_id: int = _session.docked_station_id()
				if docked_station_id >= 0:
					_connection.send_disassemble_ship_command(_player_ship_id, docked_station_id)
		InventoryRow.ACTION_BUILD_TOGGLE:
			## No command sent -- this only expands/collapses the ship-type
			## picker rows below it, then forces an immediate panel redraw
			## (there's no new PlayerLoadout snapshot to trigger one).
			_hud_surface.toggle_build_picker(
				_loadout.modules(),
				_loadout.inventory(),
				_loadout.station_inventory(),
				_loadout.owned_ships(),
				_buildable_ship_types)
		InventoryRow.ACTION_BUILD_SHIP_TYPE:
			## Dedicated button alongside the existing [B] key (Phase 9B task
			## 10), but lets the player pick which buildable type instead of
			## always sending the hard-coded BUILDABLE_SHIP_TYPE_ID.
			if _player_ship_id >= 0:
				var docked_station_id: int = _session.docked_station_id()
				if docked_station_id >= 0:
					_connection.send_build_packaged_ship_command(
						_player_ship_id, docked_station_id, row.ship_type_id)


## Right-click on a SHIP CARGO row moves the whole stack to the docked
## station's inventory (ADR-0034 9B). Uniform across item types (Module,
## ScrapMetal) per the user's explicit preference for a single straightforward
## right-click gesture rather than per-type UI carve-outs.
func _handle_inventory_row_right_click(row: InventoryRow) -> void:
	if row.source != InventoryRow.SOURCE_SHIP_CARGO:
		return
	if _player_ship_id < 0:
		return
	var docked_station_id: int = _session.docked_station_id()
	if docked_station_id < 0:
		return
	if row.item_id != null:
		_connection.send_transfer_to_station_command(
			_player_ship_id, docked_station_id, row.item_id)


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
	var target_column: String = _hud_surface.inventory_panel_column_at(release_pos)
	_handle_inventory_row_drop(row, target_column, release_pos)


## Dispatch matrix for a drag that ended in `target_column`. Right-click
## transfer and plain-click fit/unfit/etc. are untouched by this -- drag is
## an additive path to the same commands, same "keep both interaction paths"
## precedent as the Build/Disassemble buttons-plus-keys work.
func _handle_inventory_row_drop(row: InventoryRow, target_column: String, release_pos: Vector2) -> void:
	if target_column == "":
		return
	if row.source == InventoryRow.SOURCE_FITTED and target_column == InventoryRow.SOURCE_FITTED:
		## Reordering needs the specific row dropped on, not just the column
		## -- mismatched slot kinds (e.g. dropping a High module onto a Mid
		## row) are a no-op, since a module can't change slot kind by reorder.
		var target_row: InventoryRow = _hud_surface.inventory_panel_row_at(release_pos)
		if target_row == null or target_row.source != InventoryRow.SOURCE_FITTED:
			return
		if target_row.slot != row.slot or target_row.module_id == row.module_id:
			return
		if _player_ship_id >= 0 and _session.is_docked():
			_connection.send_reorder_fitted_module_command(
				_player_ship_id, row.slot, row.slot_index, target_row.slot_index)
		return
	if target_column == row.source:
		return
	match row.source:
		InventoryRow.SOURCE_SHIP_CARGO:
			match target_column:
				InventoryRow.SOURCE_FITTED:
					if row.item_id != null and row.item_id.is_module() \
							and _player_ship_id >= 0 and _session.is_docked():
						_connection.send_fit_module_command(
							_player_ship_id, row.item_id.module_id() as int, row.slot)
				InventoryRow.SOURCE_STATION:
					_handle_inventory_row_right_click(row)
		InventoryRow.SOURCE_FITTED:
			match target_column:
				InventoryRow.SOURCE_SHIP_CARGO:
					if _player_ship_id >= 0 and _session.is_docked():
						_connection.send_unfit_module_command(_player_ship_id, row.module_id, row.slot)
		InventoryRow.SOURCE_STATION:
			match target_column:
				InventoryRow.SOURCE_SHIP_CARGO:
					if _player_ship_id >= 0 and row.item_id != null:
						var docked_station_id: int = _session.docked_station_id()
						if docked_station_id >= 0:
							_connection.send_transfer_from_station_command(
								_player_ship_id, docked_station_id, row.item_id)
		## else: SHIPS column, or anything else -- not a meaningful drag target.


func _toggle_module_by_index(f_index: int) -> void:
	if _player_ship_id < 0:
		return
	## F1-F8 map to active module indices 0-7 (High/Mid slots)
	var toggle: Dictionary = _loadout.toggle_at(f_index)
	if toggle.is_empty():
		return
	var mid: int = toggle["module_id"] as int
	var slot: String = toggle["slot"] as String
	var kind: String = toggle.get("kind", "") as String
	if toggle["is_active"] as bool:
		_apply_player_module_activation(mid, false, "")
		_connection.send_deactivate_module(mid, slot)
	else:
		## Weapon/Tackle/Remote-repair require a Locked target (ADR-0035/0036);
		## other kinds (self-only Active modules) must not carry one.
		var requires_target: bool = toggle.get("requires_target", false) as bool
		if requires_target and _player_lock_target < 0:
			## Sending this without a target is rejected server-side outright
			## (ADR-0035: requires_target() vs target.is_some() mismatch),
			## which the client can only observe as an instant on-then-off
			## flicker (the PlayerFitting resync correcting the optimistic
			## toggle). Refuse client-side instead so the player gets a
			## clear reason rather than a confusing flicker.
			_jump_notice = "No target locked"
			_jump_notice_timer = 2.0
			return
		var target_id: int = _player_lock_target if requires_target else -1
		if requires_target and target_id >= 0 and _player_ship_id >= 0:
			if not _ships.has(target_id) or not _ships.has(_player_ship_id):
				## Locked target has left AoI (ADR-0019: Lock survives AoI
				## leave via WorldSession.remove_ship(clear_lock=false))
				## -- its node is gone so there is no position to check range
				## against. A target outside AoI is certain to be beyond any
				## module's effective range, so refuse here too rather than
				## falling through to an optimistic send that the server
				## rejects a moment later (the same on-then-off flicker the
				## visible-target check below exists to prevent).
				_jump_notice = "Target out of range"
				_jump_notice_timer = 2.0
				return
			## Same idea as the missing-lock guard above, but for range
			## (ADR-0035): the server rejects activation against a Locked
			## but out-of-range target outright, which otherwise shows the
			## exact same instant on-then-off flicker. Mirrors
			## range_gate.rs's effective_range_from_stats(), using the
			## module's own contribution (not yet active) plus every
			## already-active module of the same family.
			var range: float = toggle.get("effective_range", -1.0) as float
			if range >= 0.0:
				var dist_u: float = (_ships[_player_ship_id] as Node3D).global_position.distance_to(
					(_ships[target_id] as Node3D).global_position) / _world.render_scale()
				if dist_u > range:
					_jump_notice = "Target out of range"
					_jump_notice_timer = 2.0
					return
		_apply_player_module_activation(mid, true, "")
		_connection.send_activate_module(mid, slot, target_id)

func _set_as_player_ship(p_ship_id: int, ship: Node3D) -> void:
	_player_ship_id = p_ship_id
	_presentation.attach_player_ship(ship, _weapon_range, _weapon_falloff)

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
		"set_velocity", _velocity_components_to_vec3(velocity), tick)
	_sync_session_state()
func _handle_motion_correction(correction: MotionCorrectionPresentation) -> void:
	if correction.ship_id != _player_ship_id or not _ships.has(correction.ship_id):
		return
	(_ships[correction.ship_id] as Node3D).call(
		"reconcile_motion",
		correction.position,
		_velocity_components_to_vec3(correction.velocity),
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
	var speed_str: String = "-"
	if _player_ship_id >= 0 and _ships.has(_player_ship_id):
		var spd: float = (_ships[_player_ship_id] as Node3D).call("get_speed_server") as float
		speed_str = UnitFormat.format_speed(spd * METERS_PER_UNIT)
	var target_known: bool = _ships.has(_player_lock_target)
	var dist_text: String = "—"
	if target_known and _player_ship_id >= 0 and _ships.has(_player_ship_id):
		var dist_m: float = (_ships[_player_ship_id] as Node3D).global_position.distance_to(
			(_ships[_player_lock_target] as Node3D).global_position) / _world.render_scale()
		dist_text = UnitFormat.format_distance(dist_m * METERS_PER_UNIT)
	var target_hp: Variant = _session.ship_health(_player_lock_target)

	var jump_line  : String = ""
	if _nearby_gate_id >= 0:
		jump_line = "\n[J] Jump Gate #%d" % _nearby_gate_id
	if _jump_notice != "":
		jump_line += "\n" + _jump_notice

	var station_line: String = ""
	if _session.is_docked():
		var docked_station_id: int = _session.docked_station_id()
		var docked_station_name: String = _session.docked_station_name()
		var docked_name := docked_station_name if not docked_station_name.is_empty() else "Station #%d" % docked_station_id
		if _player_ship_id >= 0:
			station_line = (
				"\nDocked: %s\n[U] Undock  [B] Build Magpie\n[Y] Disassemble ship  [X] Disembark"
				% docked_name
			)
		else:
			## Disembarked (ADR-0037): still docked, but no ship is active.
			## No client UI yet to pick among owned ships (roadmap.md §12
			## task 10), so this just confirms the state without an action hint.
			station_line = "\nDisembarked at: %s\n(no active ship)" % docked_name
	elif not _nearby_station_ids.is_empty():
		var nearest_name: String = _station_name(_nearby_station_ids[0])
		if _nearby_station_ids.size() == 1:
			station_line = "\nNearby: %s\n[D] Dock at %s" % [nearest_name, nearest_name]
		else:
			var names: Array[String] = []
			for sid: int in _nearby_station_ids:
				names.append(_station_name(sid))
			station_line = "\nNearby: %s\n[D] Dock at %s (nearest)" % [", ".join(names), nearest_name]

	## Approach / warp target selection (ADR-0015 / ADR-0022 / ADR-0025).
	var keep_at_range_hint: String = "\n[O] Orbit  [K] Keep at %.0f km  ([/]  adjust)" % _keep_at_range_km

	var approach_line: String = ""
	var selected_gate_id: int = _interaction.selected_gate_id()
	var selected_body_id: int = _interaction.selected_body_id()
	var selected_target_id: int = _interaction.selected_target_id()
	if selected_gate_id >= 0:
		approach_line = "\n[A] Approach Gate #%d" % selected_gate_id + keep_at_range_hint
		## Warp is only valid beyond the minimum warp distance (ADR-0022).
		var gate_dist: float = _selected_gate_distance()
		if gate_dist >= _client_rules.min_warp_distance():
			approach_line += "\n[W] Warp  [J] Warp+Jump"
		elif gate_dist >= 0.0:
			approach_line += "\n[W] too close to warp"
	elif selected_body_id >= 0:
		## Look up body name for HUD.
		var body_name: String = "Body #%d" % selected_body_id
		for entry: Variant in _bodies:
			var body: CelestialBodyRecord = entry as CelestialBodyRecord
			if body.body_id == selected_body_id:
				body_name = body.name
				break
		approach_line = "\n[W] Warp to %s" % body_name
	elif selected_target_id >= 0:
		approach_line = "\n[A] Approach #%d" % selected_target_id + keep_at_range_hint

	var health: ShipHealth = _session.player_health()
	var cap: CapacitorStatus = _session.capacitor_status()
	_hud_surface.render({
		"connected": _connection.is_connected_to_server(),
		"ship_type_name": _session.player_ship_type_name(),
		"system_name": _session.current_system_name(),
		"speed": speed_str,
		"player_ship_id": _player_ship_id,
		"shield": health.shield,
		"max_shield": health.max_shield,
		"armor": health.armor,
		"max_armor": health.max_armor,
		"hull": health.hull,
		"max_hull": health.max_hull,
		"cap_current": cap.current,
		"cap_max": cap.max,
		"lock_target": _player_lock_target,
		"target_known": target_known,
		"target_distance": dist_text,
		"target_hp": target_hp,
		"modules": _loadout.modules(),
		"stats_text": (
			"Ships: %d\nTick: %d%s%s\n\n[Click] Select  [DoubleClick] Thrust\n[RightClick] Lock%s"
			% [_ships.size(), _session.current_tick(), approach_line, station_line, jump_line]
		),
	})

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
