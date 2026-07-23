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
const WorldSessionScript = preload("res://scripts/world_session.gd")
const WorldInteractionScript = preload("res://scripts/world_interaction.gd")
const InventoryRow = preload("res://scripts/inventory_row.gd")
const WORLD_SCALE : float = 0.1   ## Server-to-Godot coordinate scale factor
const MIN_WARP_DISTANCE : float = 3000.0  ## Server units. WarpCommand is rejected for gates closer than this (ADR-0022).
## Unit-to-metre scale: real metres = (units/tick or units) * METERS_PER_UNIT,
## fed into UnitFormat for display (m/s, km/s, AU/s, ... -- whichever reads
## best at the given magnitude). Change this one constant to rescale all
## displayed speeds and distances.
const METERS_PER_UNIT : float = 1.0
const CLIENT_TICKS_PER_SEC : float = 10.0

const BUILDABLE_SHIP_TYPE_ID : int = 7

var _cap_tick_accumulator : float = 0.0

# -- Internal state -----------------------------------------------------------

var _session := WorldSessionScript.new()
var _interaction := WorldInteractionScript.new()
var _hud_surface := HudSurfaceScript.new()
var _market_surface := MarketSurfaceScript.new()
var _presentation := WorldPresentationScript.new()
var _ships                 : Dictionary = _session.ships
var _player_ship_id        : int        = -1
var _player_ship_type_name : String     = ""
var _event_count     : int        = 0
var _current_tick    : int        = 0
## 3-layer HP tracking (Shield / Armor / Hull)
var _player_shield     : float = -1.0
var _player_armor      : float = -1.0
var _player_hull       : float = -1.0
var _player_max_shield : float = 500.0
var _player_max_armor  : float = 300.0
var _player_max_hull   : float = 200.0
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

## Per-ship HP: { ship_id: {shield, armor, hull} }
var _ship_hp : Dictionary = _session.ship_hp

## Duel mode: opponent player ship IDs (populated from InitialState is_player flag)
var _opponent_ship_ids : Array = _session.opponent_ship_ids
## PlayerLoadout is a GDExtension class (dawn-client-gdext, ADR-0039/ADR-0040)
## -- no preload needed, same as any other globally registered class.
var _loadout := PlayerLoadout.new()

## Client-side capacitor simulation (mirrors server CapacitorSystem logic).
## Populated from InitialState (cap_max, cap_recharge_per_tick) and
## PlayerLoadout (cap_cost_per_cycle, cycle_time_ticks per module).
## Corrected by ModuleDeactivated events (cap-forced OFF).
var _cap_current      : float = -1.0   ## -1 = not yet received
var _cap_max          : float = 500.0
var _cap_recharge     : float = 10.0   ## GJ per tick

var _weapon_range     : float  = 0.0   ## optimal range (u), recalculated on fitting change
var _weapon_falloff   : float  = 0.0   ## falloff range (u)

## Navigation map for the *current* Sector, received from the server in the
## InitialState message (ADR-0009/0025). No longer hard-coded: the server owns
## the galaxy (data/galaxy.toml) and is the single source of truth.
##   _gates : [{gate_id:int, position:Vector3 (server coords),
##             activation_radius:float, to_system_name:String}]
##   _bodies: [{body_id:int, kind:String, name:String,
##             position:Vector3 (server coords), radius:float, spectral_type:float}]
var _gates        : Array      = _session.gates
var _stations     : Array      = _session.stations
var _bodies       : Array      = _session.bodies
## Buildable Packaged Ship catalog (ADR-0034 9B): [{ship_type_id:int, name:String}].
var _buildable_ship_types : Array = _session.buildable_ship_types
## Star System id -> name, used to resolve StarSystemChanged events.
var _system_names : Dictionary = _session.system_names

## Placeholder until InitialState arrives -- never a real system name (was
## hardcoded to "Alpha", which looked like live data while still CONNECTING).
var _current_system_name : String = "Unknown"
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
	_connection.event_received.connect(_on_event_received)
	_connection.motion_correction_received.connect(_handle_motion_correction)
	_connection.connection_changed.connect(_on_connection_changed)
	_connection.welcomed.connect(_on_welcomed)
	_connection.initial_state_received.connect(_on_initial_state)
	_connection.player_fitting_received.connect(_on_player_fitting)
	_connection.module_activated.connect(_on_module_activated)
	_connection.module_deactivated.connect(_on_module_deactivated)
	_connection.market_snapshot_received.connect(_on_market_snapshot)
	_presentation.build(self, _camera, _warp_tunnel, _gates_root, _bodies_root, _world, WORLD_SCALE)
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
## origin and is the only place server<->Godot conversions happen. Preloaded
## (not referenced by global class_name) so headless tests that load main.gd
## resolve it without the editor's script-class cache.
const WorldSpaceScript = preload("res://scripts/world_space.gd")
var _world := WorldSpaceScript.new()

## Real-unit (m/s, km/s, AU/s, ...) display formatting (ADR-0029 §1.5: single
## conversion module). Static methods only -- preloaded rather than referenced
## by global class_name for the same headless-test-cache reason as WorldSpace.
const UnitFormat = preload("res://scripts/unit_format.gd")

## Converts a server-space position (Y-up, +Z) into Godot world space (Y-up,
## -Z), relative to the floating origin and scaled by WORLD_SCALE. Shared by
## gate/body marker spawning and gate picking, which all place a Node3D at a
## server-given position. Thin wrapper so it can be passed as a Callable.
func _server_to_godot_pos(p: Vector3) -> Vector3:
	return _world.to_godot(p)

## Reads a {x,y,z} sub-dictionary -- the wire format for every server-space
## Vector3 field (position/entry_pos/...) -- into a Vector3. Shared by every
## event/state handler that parses a position out of a payload dict.
func _vec3_from_dict(d: Dictionary, key: String) -> Vector3:
	return WorldSessionScript.vec3_from_dict(d, key)

func _position_components_from_dict(d: Dictionary, key: String) -> PackedFloat64Array:
	return WorldSessionScript.position_components_from_dict(d, key)

## Reads a {dx,dy,dz} velocity sub-dictionary from a wire payload.
func _velocity_from_dict(d: Dictionary, key: String = "velocity") -> Vector3:
	var velocity: Dictionary = d.get(key, {}) as Dictionary
	return Vector3(
		velocity.get("dx", 0.0) as float,
		velocity.get("dy", 0.0) as float,
		velocity.get("dz", 0.0) as float)

## Tracks whether the player ship is within activation range of a Jump Gate
## (ADR-0009). Distance is computed in server units (Godot units / WORLD_SCALE).
func _update_gate_proximity() -> void:
	_nearby_gate_id = -1
	if _player_ship_id < 0 or not _ships.has(_player_ship_id):
		return
	var ship_pos: Vector3 = _world.to_server((_ships[_player_ship_id] as Node3D).global_position)
	for gate: Variant in _gates:
		var g: Dictionary = gate as Dictionary
		var gate_pos: Vector3 = g.get("position", Vector3.ZERO) as Vector3
		if ship_pos.distance_to(gate_pos) <= (g.get("activation_radius", 0.0) as float):
			_nearby_gate_id = g.get("gate_id", -1) as int
			return


## Every station currently within docking range, nearest first. Usually at
## most one (stations shouldn't be placed close enough to overlap docking
## radii), but ranked by distance rather than array order in case they ever
## are, so [D] docks at the one the player is actually closest to.
func _update_station_proximity() -> void:
	_nearby_station_ids.clear()
	if _player_ship_id < 0 or not _ships.has(_player_ship_id):
		return
	var ship_pos: Vector3 = _world.to_server((_ships[_player_ship_id] as Node3D).global_position)
	var in_range: Array[Dictionary] = []
	for station_entry: Variant in _stations:
		var station: Dictionary = station_entry as Dictionary
		var station_pos: Vector3 = station.get("position", Vector3.ZERO) as Vector3
		var dist: float = ship_pos.distance_to(station_pos)
		if dist <= (station.get("docking_radius", 0.0) as float):
			in_range.append({"station_id": station.get("station_id", -1) as int, "distance": dist})
	in_range.sort_custom(func(a: Dictionary, b: Dictionary) -> bool:
		return (a.distance as float) < (b.distance as float))
	for entry: Dictionary in in_range:
		_nearby_station_ids.append(entry.station_id as int)


## Display name for a station_id, falling back to "Station #N" if unnamed
## or not found in the galaxy map (e.g. between InitialState and StarMap sync).
func _station_name(station_id: int) -> String:
	for entry: Variant in _stations:
		var station: Dictionary = entry as Dictionary
		if (station.get("station_id", -1) as int) == station_id:
			var name: String = station.get("name", "") as String
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
		var dock_status: Dictionary = _session.dock_status()
		var nearest_station_id: int = _nearby_station_ids[0] if not _nearby_station_ids.is_empty() else -1
		var action: Dictionary = _interaction.resolve_key_action(
			key.keycode,
			_player_ship_id,
			_nearby_gate_id,
			nearest_station_id,
			dock_status.get("docked_station_id", -1) as int)
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
	var ship_server: Vector3 = _world.to_server((_ships[_player_ship_id] as Node3D).global_position)
	for gate: Variant in _gates:
		var g: Dictionary = gate as Dictionary
		if (g.get("gate_id", -1) as int) != selected_gate_id:
			continue
		var gpos: Vector3 = g.get("position", Vector3.ZERO) as Vector3
		return ship_server.distance_to(gpos)
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

func _on_event_received(payload: Dictionary) -> void:
	_session.event_count += 1
	_sync_session_state()
	var event_type: String = payload.get("type", "") as String
	match event_type:
		"ShipSpawned"      : _handle_ship_spawned(payload)
		"VelocityChanged"  : _handle_velocity_changed(payload)
		"ShipDespawned"    : _handle_ship_despawned(payload)
		"ShipDocked"       : _handle_ship_docked(payload)
		"ShipUndocked"     : _handle_ship_undocked(payload)
		"DamageTaken"   : _handle_damage_taken(payload)
		"RepairApplied" : _handle_repair_applied(payload)
		"ShipDestroyed" : _handle_ship_destroyed(payload)
		"TargetLocked"  : _handle_target_locked(payload)
		"LockLost"      : _handle_lock_lost(payload)
		"JumpGateUsed"      : _handle_jump_gate_used(payload)
		"StarSystemChanged" : _handle_star_system_changed(payload)
		"AoiEnter"          : _handle_aoi_enter(payload)
		"AoiLeave"          : _handle_aoi_leave(payload)
		"PositionSnap"      : _handle_position_snap(payload)

# -- Position snap (ADR-0029) -------------------------------------------------

## Authoritative absolute-position snap (server → client), e.g. on warp arrival
## after an anchor rebase. The client maps the server-absolute position through
## the CURRENT floating origin and snaps the ship there, correcting the large
## dead-reckoning drift a true-AU warp accumulates. Supersedes the client's
## pre-computed (and now origin-stale) warp snap for body warps.
func _handle_position_snap(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	if not _ships.has(ship_id):
		return
	var server_pos := _position_components_from_dict(p, "position")
	if ship_id == _player_ship_id:
		# A warp crosses ~1 AU but the player's visual ship lagged behind (its
		# warp speed was capped). Correcting that by moving the ship would make
		# the camera pan/swing. Instead, keep the ship (and camera) exactly where
		# they are on screen and re-anchor the floating origin so that this same
		# Godot position now represents the authoritative arrival `server_pos`:
		# new_origin = server_pos - (ship's server-space offset from the origin).
		var pg: Vector3 = _ship_position(_ships[ship_id] as Node3D)
		var new_origin := PackedFloat64Array([
			server_pos[0] - pg.x / WORLD_SCALE,
			server_pos[1] - pg.y / WORLD_SCALE,
			server_pos[2] + pg.z / WORLD_SCALE])
		_presentation.apply_origin_rebase_components(new_origin, true, _player_ship_id, _ships)
	else:
		_set_ship_position(_ships[ship_id] as Node3D, _world.to_godot_components(
			server_pos[0], server_pos[1], server_pos[2]))
	var ship := _ships[ship_id] as Node3D
	ship.call("reset_motion", _ship_position(ship), Vector3.ZERO, _current_tick)

## Docking is authoritative server state. The server stops the ship
## immediately, but without an explicit client event the ship_controller keeps
## integrating the last VelocityChanged it saw and visually drifts. Treat a
## ShipDocked like a motion snap-to-station from the client's perspective:
## zero residual velocity/thrust at once.
func _handle_ship_docked(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	var station_id: int = p.get("station_id", -1) as int
	var tick: int = p.get("tick", 0) as int
	if not _ships.has(ship_id):
		return
	for entry: Variant in _stations:
		var station: Dictionary = entry as Dictionary
		if (station.get("station_id", -1) as int) != station_id:
			continue
		_set_ship_position(_ships[ship_id] as Node3D, _server_to_godot_pos(
			station.get("position", Vector3.ZERO) as Vector3
		))
		if ship_id == _player_ship_id:
			_session.apply_dock_event(ship_id, station_id, station.get("name", "") as String, tick)
			_sync_session_state()
		break
	_stop_ship_motion(ship_id)

func _handle_ship_undocked(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	if ship_id == _player_ship_id:
		var station_id: int = p.get("station_id", -1) as int
		_nearby_station_ids.clear()
		if station_id >= 0:
			_nearby_station_ids.append(station_id)
		_session.apply_undock_event(ship_id, p.get("tick", 0) as int)
		_sync_session_state()

func _stop_ship_motion(ship_id: int) -> void:
	if not _ships.has(ship_id):
		return
	var ship := _ships[ship_id] as Node3D
	ship.call("reset_motion", _ship_position(ship), Vector3.ZERO, _current_tick)

func _ship_position(ship: Node3D) -> Vector3:
	return ship.global_position if ship.is_inside_tree() else ship.position

func _set_ship_position(ship: Node3D, godot_pos: Vector3) -> void:
	if ship.is_inside_tree():
		ship.global_position = godot_pos
	else:
		ship.position = godot_pos

# -- Jump Gate (ADR-0009) -----------------------------------------------------

## Ship passed through a Jump Gate -- teleport to entry_pos.
func _handle_jump_gate_used(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	if not _ships.has(ship_id):
		return
	var entry_pos: Vector3 = _vec3_from_dict(p, "entry_pos")
	var tick: int = p.get("tick", _current_tick) as int
	(_ships[ship_id] as Node3D).call("update_target", _world.to_godot(entry_pos), tick)
	if ship_id == _player_ship_id:
		_jump_notice       = "Jumped via Gate #%d" % (p.get("gate_id", 0) as int)
		_jump_notice_timer = 3.0

## Ship moved to a different star system -- show HUD notification.
func _handle_star_system_changed(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	var to_system: int = p.get("to_system", 0) as int
	var result: Dictionary = _session.system_changed(ship_id, to_system)
	_sync_session_state()
	if result.get("changed_player", false) as bool:
		var to_name: String = result.get("system_name", "System %d" % to_system) as String
		_jump_notice         = "Entered %s system" % to_name
		_jump_notice_timer   = 3.0
		_interaction.clear_navigation_selection()
		## Gate / body markers refresh from the next InitialState (sent when the
		## client reconnects to the destination node after the Redirect).

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
	item_type: String,
	module_id: int,
	ship_type_id: int,
	side: String,
	price: int,
	quantity: int
) -> void:
	if _player_ship_id < 0 or not _session.is_docked():
		return
	_connection.send_market_place_order_command(
		_player_ship_id, item_type, module_id, ship_type_id, side, price, quantity)


func _on_market_cancel_order(order_id: int) -> void:
	if not _session.is_docked():
		return
	_connection.send_market_cancel_order_command(order_id)


func _on_market_snapshot(snapshot: Dictionary) -> void:
	_market_surface.apply_snapshot(snapshot)

## InitialState received: ingest the Sector's navigation map, then spawn all
## ship nodes in one pass. ShipSpawned events are not sent in Phase 5;
## InitialState handles initialization.
func _on_initial_state(state: Dictionary) -> void:
	_clear_all_ships()  ## Reset on reconnect
	_hud_surface.hide_duel_result()
	_ingest_star_map(state)

	for ship_data: Variant in (state.get("ships", []) as Array):
		_spawn_ship_from_data(ship_data as Dictionary)

## Store the server-provided navigation map (system names, this Sector's gates
## and bodies) and rebuild the gate / body markers from it. This replaces the
## previously hard-coded JUMP_GATES / CELESTIAL_BODIES / STAR_SYSTEM_NAMES.
func _ingest_star_map(state: Dictionary) -> void:
	_session.ingest_navigation(state)
	_sync_session_state()
	_presentation.respawn_navigation_markers(
		_gates,
		_bodies,
		_stations,
		_server_to_godot_pos,
		Callable(_interaction, "clear_navigation_selection")
	)

## Instantiate a ship scene at `pos` (server-space) and return the node.
## WorldSession owns registry insertion; main.gd only creates the scene node.
func _instantiate_ship(sid: int, pos: Vector3) -> Node3D:
	return _instantiate_ship_at_godot(sid, _world.to_godot(pos))

func _instantiate_ship_at_godot(sid: int, godot_pos: Vector3) -> Node3D:
	var ship: Node3D = SHIP_SCENE.instantiate() as Node3D
	_ships_root.add_child(ship)
	ship.call("initialize", sid, godot_pos)
	ship.name = "Ship_%d" % sid
	return ship

## Materialize one ship node from a ship-state dict. Shared by InitialState and
## AoiEnter (ADR-0019). Skips ships already present.
func _spawn_ship_from_data(d: Dictionary) -> void:
	var sid: int = d.get("ship_id", 0) as int
	if _ships.has(sid):
		return
	var server_pos := _position_components_from_dict(d, "position")
	var ship: Node3D = _instantiate_ship_at_godot(sid, _world.to_godot_components(
		server_pos[0], server_pos[1], server_pos[2]))
	ship.call(
		"configure_motion",
		d.get("max_speed", 500.0) as float,
		d.get("mass", 10_000_000.0) as float,
		d.get("inertia_modifier", 0.3) as float,
		_velocity_from_dict(d),
		_current_tick)
	var result: Dictionary = _session.register_ship(sid, ship, d, _connection.ship_id)
	_sync_session_state()
	if result.get("became_player", false) as bool:
		_set_as_player_ship(sid, ship)

## AoI: a ship entered the player's neighborhood -- materialize it (ADR-0019).
func _handle_aoi_enter(p: Dictionary) -> void:
	var ship: Dictionary = p.get("ship", {}) as Dictionary
	if not ship.is_empty():
		_spawn_ship_from_data(ship)

## AoI: a ship left the player's neighborhood -- remove it locally with no death
## effect (it is still alive elsewhere, just out of view / ADR-0019).
func _handle_aoi_leave(p: Dictionary) -> void:
	var sid: int = p.get("ship_id", 0) as int
	## clear_lock=false: the ship is still alive server-side, just outside
	## this player's AoI radius (ADR-0019) -- Lock has no distance-based
	## expiry (lock.rs), so clearing player_lock_target here would desync
	## from the server and strand the lock forever (see world_session.gd).
	var result: Dictionary = _session.remove_ship(sid, false)
	_sync_session_state()
	if not (result.get("removed", false) as bool):
		return
	_interaction.clear_target_if_matches(sid)
	var ship: Node3D = result.get("node") as Node3D
	ship.queue_free()

## Sent on connect and again after every Fit/Unfit (ADR-0032), so the panel
## and module bar always reflect the server's authoritative fitting state --
## including a rejected Fit/Unfit attempt reverting visibly.
func _on_player_fitting(bytes: PackedByteArray) -> void:
	## connection.gd hands the raw postcard bytes (ADR-0042), not a parsed
	## Dictionary: PlayerLoadout.apply_wire_bytes decodes them directly into
	## typed Rust state, with no lossy Dictionary/JSON round-trip in between.
	_loadout.apply_wire_bytes(bytes)
	_apply_loadout_side_effects()

## Everything `_on_player_fitting` does after the wire decode itself, split
## out so tests can drive it via `_loadout.apply_payload()` (test/debug-only
## JSON fixture path) without needing real postcard bytes.
func _apply_loadout_side_effects() -> void:
	## Disembark/SelectActiveShip/Assemble (ADR-0037) can change which ship
	## is active independently of any command this client sent. Only follow
	## it when it's -1 (no active ship) or a ship this client already knows
	## about -- switching to a *different* owned ship this client has never
	## rendered would need to spawn it first, which isn't covered here
	## (docs/architecture/ownership.md §8). Both branches route through
	## WorldPresentation (attach for a known ship, detach for none) instead of
	## a bare _player_ship_id assignment -- regression (fixed twice already):
	## a bookkeeping-only update leaves the camera/material/tactical-overlay
	## out of sync with which ship is actually active.
	var new_active_ship_id: int = _loadout.active_ship_id()
	if new_active_ship_id != _player_ship_id \
			and (new_active_ship_id < 0 or _ships.has(new_active_ship_id)):
		_session.player_ship_id = new_active_ship_id
		if new_active_ship_id >= 0:
			_set_as_player_ship(new_active_ship_id, _ships[new_active_ship_id] as Node3D)
		else:
			_player_ship_id = new_active_ship_id
			_presentation.detach_player_ship()

	var dock_status: Dictionary = _loadout.dock_status()
	_session.apply_dock_fitting(
		dock_status.get("docked_station_id", -1) as int,
		dock_status.get("docked_station_name", "") as String,
		_loadout.tick()
	)
	_sync_session_state()
	if not _session.is_docked() and _market_surface.is_open():
		_market_surface.set_open(false)
	if _session.is_docked() and _player_ship_id >= 0:
		_stop_ship_motion(_player_ship_id)
	var snapshot: Dictionary = _loadout.hud_snapshot()
	_hud_surface.set_player_fitting(
		snapshot.get("modules", []) as Array,
		snapshot.get("inventory", []) as Array,
		snapshot.get("station_inventory", []) as Array,
		snapshot.get("owned_ships", []) as Array,
		_buildable_ship_types)
	_market_surface.set_cargo(snapshot.get("inventory", []) as Array)
	_recalc_weapon_range()

func _recalc_weapon_range() -> void:
	var ranges: Dictionary = _loadout.weapon_ranges()
	_weapon_range = ranges["optimal"] as float
	_weapon_falloff = ranges["falloff"] as float
	_presentation.update_tactical_overlay_ranges(_weapon_range, _weapon_falloff)

func _on_module_activated(p_ship_id: int, p_module_id: int, _slot: String) -> void:
	if p_ship_id != _player_ship_id:
		return
	_apply_player_module_activation(p_module_id, true, "")

## reason is server-authoritative now ("cap" | "range" | "", ADR-0035), so it
## replaces the old "were we the one who sent Deactivate" heuristic, which
## always mislabelled a range-forced OFF as a capacitor exhaustion.
func _on_module_deactivated(p_ship_id: int, p_module_id: int, _slot: String, reason: String) -> void:
	if p_ship_id != _player_ship_id:
		return
	_apply_player_module_activation(p_module_id, false, reason)


func _apply_player_module_activation(module_id: int, active: bool, forced_reason: String) -> void:
	_loadout.apply_module_activation(module_id, active, forced_reason)
	_recalc_weapon_range()


## A row click in the inventory panel: "fit" sends the module's own slot kind
## (the module's `def.slot` decides where it goes -- the player makes no slot
## choice), "unfit" removes that exact fitted instance (ADR-0032).
func _handle_inventory_row_click(row: InventoryRow) -> void:
	match row.action:
		InventoryRow.ACTION_FIT:
			## ADR-0032's 2026-07-08 amendment: refitting requires being
			## docked. Guarded client-side too (not just server-side) so this
			## reads as an obvious no-op rather than a silent-failure resync,
			## the same UX lesson learned from Disassemble earlier this phase.
			if _player_ship_id >= 0 and _session.is_docked():
				_connection.send_fit_module_command(_player_ship_id, row.module_id, row.slot)
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
			var docked_station_id: int = _session.dock_status().get("docked_station_id", -1) as int
			if docked_station_id >= 0:
				_connection.send_assemble_command(docked_station_id, row.ship_type_id)
		InventoryRow.ACTION_SELECT_ACTIVE_SHIP:
			## Also no active-ship requirement -- this is how a player re-boards
			## after Disembark, or switches to a different owned ship.
			_connection.send_select_active_ship_command(row.ship_id)
		InventoryRow.ACTION_DISASSEMBLE:
			## Dedicated button alongside the existing [Y] key (Phase 9B task
			## 10) -- same command, server validates docked/undamaged/unfitted.
			if _player_ship_id >= 0:
				var docked_station_id: int = _session.dock_status().get("docked_station_id", -1) as int
				if docked_station_id >= 0:
					_connection.send_disassemble_ship_command(_player_ship_id, docked_station_id)
		InventoryRow.ACTION_BUILD_TOGGLE:
			## No command sent -- this only expands/collapses the ship-type
			## picker rows below it, then forces an immediate panel redraw
			## (there's no new PlayerLoadout snapshot to trigger one).
			var snapshot: Dictionary = _loadout.hud_snapshot()
			_hud_surface.toggle_build_picker(
				snapshot.get("modules", []) as Array,
				snapshot.get("inventory", []) as Array,
				snapshot.get("station_inventory", []) as Array,
				snapshot.get("owned_ships", []) as Array,
				_buildable_ship_types)
		InventoryRow.ACTION_BUILD_SHIP_TYPE:
			## Dedicated button alongside the existing [B] key (Phase 9B task
			## 10), but lets the player pick which buildable type instead of
			## always sending the hard-coded BUILDABLE_SHIP_TYPE_ID.
			if _player_ship_id >= 0:
				var docked_station_id: int = _session.dock_status().get("docked_station_id", -1) as int
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
	var docked_station_id: int = _session.dock_status().get("docked_station_id", -1) as int
	if docked_station_id < 0:
		return
	_connection.send_transfer_to_station_command(
		_player_ship_id, docked_station_id, row.item_type, row.module_id, row.ship_type_id)


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
					if row.item_type == "Module" and _player_ship_id >= 0 and _session.is_docked():
						_connection.send_fit_module_command(_player_ship_id, row.module_id, row.slot)
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
					if _player_ship_id >= 0:
						var docked_station_id: int = _session.dock_status().get("docked_station_id", -1) as int
						if docked_station_id >= 0:
							_connection.send_transfer_from_station_command(
								_player_ship_id, docked_station_id, row.item_type, row.module_id,
								row.ship_type_id)
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
		if requires_target and _session.player_lock_target < 0:
			## Sending this without a target is rejected server-side outright
			## (ADR-0035: requires_target() vs target.is_some() mismatch),
			## which the client can only observe as an instant on-then-off
			## flicker (the PlayerFitting resync correcting the optimistic
			## toggle). Refuse client-side instead so the player gets a
			## clear reason rather than a confusing flicker.
			_jump_notice = "No target locked"
			_jump_notice_timer = 2.0
			return
		var target_id: int = _session.player_lock_target if requires_target else -1
		if requires_target and target_id >= 0 and _player_ship_id >= 0:
			if not _ships.has(target_id) or not _ships.has(_player_ship_id):
				## Locked target has left AoI (ADR-0019: Lock survives AoI
				## leave via world_session.gd remove_ship(clear_lock=false))
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
					(_ships[target_id] as Node3D).global_position) / WORLD_SCALE
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

func _handle_ship_spawned(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	if _ships.has(ship_id):
		return

	var ship: Node3D = _instantiate_ship(ship_id, _vec3_from_dict(p, "position"))
	var result: Dictionary = _session.register_ship(ship_id, ship, p, _connection.ship_id)
	_sync_session_state()

	## If this ship matches the player_id from Welcome, set it as the player ship
	if result.get("became_player", false) as bool:
		_set_as_player_ship(ship_id, ship)

func _handle_velocity_changed(p: Dictionary) -> void:
	var ship_id : int = p.get("ship_id", 0) as int
	if not _ships.has(ship_id):
		return

	var server_vel := _velocity_from_dict(p)
	(_ships[ship_id] as Node3D).call("set_velocity", server_vel)

	## Warp arrival is corrected by the server's PositionSnap (ADR-0029), not by
	## client-side dead-reckoning detection.

	var tick: int = p.get("tick", 0) as int
	_session.advance_tick_from_event(tick, _loadout)
	_sync_session_state()

## Owner-only absolute correction for Rust client-side prediction (ADR-0043).
## Other ships continue to use the event-driven dead-reckoning path.
func _handle_motion_correction(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	if ship_id != _player_ship_id or not _ships.has(ship_id):
		return
	var tick: int = p.get("tick", 0) as int
	var server_pos := _position_components_from_dict(p, "position")
	(_ships[ship_id] as Node3D).call(
		"reconcile_motion",
		_world.to_godot_components(
			server_pos[0],
			server_pos[1],
			server_pos[2]),
		_velocity_from_dict(p),
		tick)


func _handle_ship_despawned(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	var result: Dictionary = _session.remove_ship(ship_id)
	_sync_session_state()
	if not (result.get("removed", false) as bool):
		return
	var ship: Node3D = result.get("node") as Node3D
	ship.queue_free()
	_interaction.clear_target_if_matches(ship_id)

func _handle_damage_taken(p: Dictionary) -> void:
	var result: Dictionary = _session.apply_hp_event(p)
	_sync_session_state()
	var ship_id: int = result.get("ship_id", 0) as int
	## Flash red on any ship that takes damage (visual hit feedback)
	if _ships.has(ship_id):
		(_ships[ship_id] as Node3D).call("flash_damage")

func _handle_repair_applied(p: Dictionary) -> void:
	var result: Dictionary = _session.apply_hp_event(p)
	_sync_session_state()
	var ship_id: int = result.get("ship_id", 0) as int
	if _ships.has(ship_id):
		(_ships[ship_id] as Node3D).call("flash_repair")

func _handle_ship_destroyed(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	var result: Dictionary = _session.destroy_ship(ship_id)
	_sync_session_state()
	if not (result.get("destroyed", false) as bool):
		return
	var ship: Node3D = result.get("node") as Node3D
	_interaction.clear_target_if_matches(ship_id)
	## Play destruction effect (queue_free happens inside play_destroy_effect)
	ship.call("play_destroy_effect")
	if result.get("destroyed_player", false) as bool:
		_hud_surface.show_duel_result(false)  ## DEFEAT
	elif result.get("destroyed_opponent", false) as bool:
		_hud_surface.show_duel_result(true)   ## VICTORY

func _handle_target_locked(p: Dictionary) -> void:
	var locker_id: int = p.get("locker_id", 0) as int
	var target_id: int = p.get("target_id", 0) as int
	## Player completed a lock
	if _session.apply_target_locked(locker_id, target_id):
		_sync_session_state()
		if _ships.has(target_id):
			(_ships[target_id] as Node3D).call("set_lock_state", "locked")
	## Locked by another ship (no visual indicator)

func _handle_lock_lost(p: Dictionary) -> void:
	var locker_id: int = p.get("locker_id", 0) as int
	var target_id: int = p.get("target_id", 0) as int
	if _session.apply_lock_lost(locker_id, target_id):
		_sync_session_state()
		if _ships.has(target_id):
			(_ships[target_id] as Node3D).call("set_lock_state", "none")

# -- HUD ----------------------------------------------------------------------

func _update_hud() -> void:
	var speed_str: String = "-"
	if _player_ship_id >= 0 and _ships.has(_player_ship_id):
		var spd: float = (_ships[_player_ship_id] as Node3D).call("get_speed_server") as float
		speed_str = UnitFormat.format_speed(spd * METERS_PER_UNIT)
	var target_known: bool = _ships.has(_player_lock_target)
	var dist_text: String = "—"
	if target_known and _player_ship_id >= 0 and _ships.has(_player_ship_id):
		var dist_m: float = (_ships[_player_ship_id] as Node3D).global_position.distance_to(
			(_ships[_player_lock_target] as Node3D).global_position) / WORLD_SCALE
		dist_text = UnitFormat.format_distance(dist_m * METERS_PER_UNIT)
	var target_hp: Dictionary = _ship_hp.get(_player_lock_target, {}) as Dictionary

	var jump_line  : String = ""
	if _nearby_gate_id >= 0:
		jump_line = "\n[J] Jump Gate #%d" % _nearby_gate_id
	if _jump_notice != "":
		jump_line += "\n" + _jump_notice

	var station_line: String = ""
	if _session.is_docked():
		var status: Dictionary = _session.dock_status()
		var docked_station_id: int = status.get("docked_station_id", -1) as int
		var docked_station_name: String = status.get("docked_station_name", "") as String
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
	var selection: Dictionary = _interaction.selection_state()
	var selected_gate_id: int = selection.get("selected_gate_id", -1) as int
	var selected_body_id: int = selection.get("selected_body_id", -1) as int
	var selected_target_id: int = selection.get("selected_target_id", -1) as int
	if selected_gate_id >= 0:
		approach_line = "\n[A] Approach Gate #%d" % selected_gate_id + keep_at_range_hint
		## Warp is only valid beyond the minimum warp distance (ADR-0022).
		var gate_dist: float = _selected_gate_distance()
		if gate_dist >= MIN_WARP_DISTANCE:
			approach_line += "\n[W] Warp  [J] Warp+Jump"
		elif gate_dist >= 0.0:
			approach_line += "\n[W] too close to warp"
	elif selected_body_id >= 0:
		## Look up body name for HUD.
		var body_name: String = "Body #%d" % selected_body_id
		for entry: Variant in _bodies:
			var b: Dictionary = entry as Dictionary
			if (b.get("body_id", -1) as int) == selected_body_id:
				body_name = b.get("name", body_name) as String
				break
		approach_line = "\n[W] Warp to %s" % body_name
	elif selected_target_id >= 0:
		approach_line = "\n[A] Approach #%d" % selected_target_id + keep_at_range_hint

	_hud_surface.render({
		"connected": _connection.is_connected_to_server(),
		"ship_type_name": _player_ship_type_name,
		"system_name": _current_system_name,
		"speed": speed_str,
		"player_ship_id": _player_ship_id,
		"shield": _player_shield,
		"max_shield": _player_max_shield,
		"armor": _player_armor,
		"max_armor": _player_max_armor,
		"hull": _player_hull,
		"max_hull": _player_max_hull,
		"cap_current": _cap_current,
		"cap_max": _cap_max,
		"lock_target": _player_lock_target,
		"target_known": target_known,
		"target_distance": dist_text,
		"target_hp": target_hp,
		"modules": _loadout.modules(),
		"stats_text": (
			"Ships: %d\nTick: %d%s%s\n\n[Click] Select  [DoubleClick] Thrust\n[RightClick] Lock%s"
			% [_ships.size(), _current_tick, approach_line, station_line, jump_line]
		),
	})

# -- Capacitor client-side simulation -----------------------------------------

## Mirror of CapacitorSystem::run() -- called once per tick elapsed.
## Keeps cap display in sync without any extra server messages.
func _simulate_cap(ticks: int) -> void:
	_session.advance_client_ticks(ticks, _loadout)
	_sync_session_state()

func _advance_client_cap_ticks(delta: float) -> void:
	if _player_ship_id < 0 or _cap_current < 0.0:
		_cap_tick_accumulator = 0.0
		return
	_cap_tick_accumulator += delta * CLIENT_TICKS_PER_SEC
	var ticks: int = int(floor(_cap_tick_accumulator))
	if ticks <= 0:
		return
	_cap_tick_accumulator -= float(ticks)
	_simulate_cap(ticks)

# -- Internal utilities -------------------------------------------------------

func _clear_all_ships() -> void:
	for ship_node: Node3D in _ships.values():
		if is_instance_valid(ship_node):
			ship_node.queue_free()
	_session.reset()
	_sync_session_state()
	_interaction.clear_selection()
	_loadout.reset()
	_cap_tick_accumulator = 0.0


func _sync_session_state() -> void:
	_ships = _session.ships
	_ship_hp = _session.ship_hp
	_opponent_ship_ids = _session.opponent_ship_ids
	_gates = _session.gates
	_stations = _session.stations
	_bodies = _session.bodies
	_buildable_ship_types = _session.buildable_ship_types
	_system_names = _session.system_names
	_player_ship_id = _session.player_ship_id
	_player_ship_type_name = _session.player_ship_type_name
	_player_shield = _session.player_shield
	_player_armor = _session.player_armor
	_player_hull = _session.player_hull
	_player_max_shield = _session.player_max_shield
	_player_max_armor = _session.player_max_armor
	_player_max_hull = _session.player_max_hull
	_player_lock_target = _session.player_lock_target
	_current_tick = _session.current_tick
	_event_count = _session.event_count
	_current_system_name = _session.current_system_name
	_cap_current = _session.cap_current
	_cap_max = _session.cap_max
	_cap_recharge = _session.cap_recharge
