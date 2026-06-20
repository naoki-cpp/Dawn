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

@onready var _connection  : Node        = $Connection
@onready var _ships_root  : Node3D      = $World/Ships
@onready var _gates_root  : Node3D      = $World/Gates
@onready var _bodies_root : Node3D      = $World/Bodies
@onready var _stats_label : Label       = $HUD/StatsLabel
@onready var _hud         : CanvasLayer = $HUD
@onready var _camera      : Camera3D   = $World/Camera3D

# -- HUD panels (built by HudManager in _ready, architecture-review-client.md C-1) --
## Top-left status panel. {conn_dot, conn_label, name_label, info_label}.
var _status_panel_refs : Dictionary = {}
## Bottom-left ship-status panel. {bar_shield, bar_armor, bar_hull, bar_cap},
## each itself {row, bar, value}.
var _ship_status_refs : Dictionary = {}
## Top-center target panel (visible only while a lock target is held).
## {panel, name_label, dist_label, bar_shield, bar_armor, bar_hull}.
var _target_panel_refs : Dictionary = {}
## Bottom-center module bar. One slot per active module, in F-key order.
## Each entry: {panel, style, name, state, module_index}.
var _module_bar   : HBoxContainer = null
var _module_slots : Array         = []

# -- Constants ----------------------------------------------------------------

const SHIP_SCENE  := preload("res://scenes/ship.tscn")
const WORLD_SCALE : float = 0.1   ## Server-to-Godot coordinate scale factor
const MIN_WARP_DISTANCE : float = 3000.0  ## Server units. WarpCommand is rejected for gates closer than this (ADR-0022).
## Warp arrival distance from target centre, as a multiple of the target's own
## radius (gate activation_radius / body radius). Gates arrive closer in (well
## inside jump range); bodies arrive further out (outside the visual sphere).
const GATE_WARP_ARRIVAL_FACTOR : float = 0.75
const BODY_WARP_ARRIVAL_FACTOR : float = 1.5
## Unit-to-meter scale: displayed m/s = (units/tick) * METERS_PER_UNIT.
## Change this one constant to rescale all displayed speeds and distances.
const METERS_PER_UNIT : float = 1.0

# -- Materials ----------------------------------------------------------------

var _player_material : StandardMaterial3D = null

# -- Internal state -----------------------------------------------------------

var _ships                 : Dictionary = {}
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
## Approach target selected by left-clicking a ship (ADR-0015). -1 = none.
var _selected_target_id : int  = -1
## Approach target selected by left-clicking a Jump Gate (ADR-0015). -1 = none.
## Mutually exclusive with _selected_target_id.
var _selected_gate_id   : int  = -1

## Per-ship HP: { ship_id: {shield, armor, hull} }
var _ship_hp : Dictionary = {}

## Duel mode: opponent player ship IDs (populated from InitialState is_player flag)
var _opponent_ship_ids : Array = []
## Duel result overlay label (created dynamically)
var _duel_result_label : Label = null

## Module slot info
## [{slot, index, module_id, name, is_active, is_active_module,
##   cap_cost_per_cycle, cycle_time_ticks, cycle_remaining}, ...]
var _player_modules : Array = []

## Client-side capacitor simulation (mirrors server CapacitorSystem logic).
## Populated from InitialState (cap_max, cap_recharge_per_tick) and
## PlayerFitting (cap_cost_per_cycle, cycle_time_ticks per module).
## Corrected by ModuleDeactivated events (cap-forced OFF).
var _cap_current      : float = -1.0   ## -1 = not yet received
var _cap_max          : float = 500.0
var _cap_recharge     : float = 10.0   ## GJ per tick

## Tactical overlay (range rings).
var _tactical_overlay : Node3D = null  ## TacticalOverlay node, parented to player ship
var _weapon_range     : float  = 0.0   ## optimal range (u), recalculated on fitting change
var _weapon_falloff   : float  = 0.0   ## falloff range (u)

## Double-click detection
var _last_click_time  : float  = -1.0
var _last_click_pos   : Vector2 = Vector2.ZERO
const DOUBLE_CLICK_SEC: float  = 0.4   ## Two clicks within this many seconds count as a double-click
const DOUBLE_CLICK_PX : float  = 10.0  ## Within this many screen pixels

## Navigation map for the *current* Sector, received from the server in the
## InitialState message (ADR-0009/0025). No longer hard-coded: the server owns
## the galaxy (data/galaxy.toml) and is the single source of truth.
##   _gates : [{gate_id:int, position:Vector3 (server coords),
##             activation_radius:float, to_system_name:String}]
##   _bodies: [{body_id:int, kind:String, name:String,
##             position:Vector3 (server coords), radius:float, spectral_type:float}]
var _gates        : Array      = []
var _bodies       : Array      = []
## Star System id -> name, used to resolve StarSystemChanged events.
var _system_names : Dictionary = {}

var _current_system_name : String = "Alpha"
var _nearby_gate_id      : int    = -1  ## -1 = no gate in range
var _selected_body_id    : int    = -1  ## -1 = no body selected
var _sky_mat             : ShaderMaterial = null  ## reference kept for sun_direction updates
var _jump_notice         : String = ""
var _jump_notice_timer   : float  = 0.0
## Pre-computed warp arrival position in server coords.
## Set at WarpCommand/auto-warp time (ship position is known then); cleared on arrival.
## Using a pre-computed position avoids relying on the dead-reckoned position (which
## drifts significantly at warp speed) to determine the snap direction on arrival.
var _player_warp_snap_pos : Vector3 = Vector3.INF
## Was the player ship at warp speed last VelocityChanged? Used to detect arrival.
var _player_was_warping   : bool    = false
const WARP_SPEED_THRESHOLD : float = 1000.0  ## Server units/tick: above this = warping

# -- Lifecycle ----------------------------------------------------------------

func _ready() -> void:
	_connection.event_received.connect(_on_event_received)
	_connection.connection_changed.connect(_on_connection_changed)
	_connection.welcomed.connect(_on_welcomed)
	_connection.initial_state_received.connect(_on_initial_state)
	_connection.player_fitting_received.connect(_on_player_fitting)
	_connection.module_activated.connect(_on_module_activated)
	_connection.module_deactivated.connect(_on_module_deactivated)
	_build_player_material()
	_setup_space_environment()
	_duel_result_label = HudManager.build_duel_result_overlay(self)
	_status_panel_refs = HudManager.build_status_panel(_hud)
	_ship_status_refs  = HudManager.build_ship_status_panel(_hud)
	_target_panel_refs = HudManager.build_target_panel(_hud)
	_module_bar = HudManager.build_module_bar(_hud)
	_update_hud()
	## Gate / body markers are spawned from the server's InitialState, not here.

func _process(delta: float) -> void:
	_update_gate_proximity()
	_update_sun_direction()
	if _jump_notice_timer > 0.0:
		_jump_notice_timer -= delta
		if _jump_notice_timer <= 0.0:
			_jump_notice = ""
	_update_hud()

## Converts a server-space position (Y-up, +Z) into Godot world space (Y-up,
## -Z), applying WORLD_SCALE. Shared by gate/body marker spawning and gate
## picking, which all place a Node3D at a server-given position.
func _server_to_godot_pos(p: Vector3) -> Vector3:
	return Vector3(p.x, p.y, -p.z) * WORLD_SCALE

## Gate/body marker spawning lives in navigation_marker_renderer.gd
## (NavigationMarkerRenderer, architecture-review-client.md C-1) -- main.gd
## only owns the live data and non-rendering state (_selected_body_id reset).

## Spawns a visual marker for every Jump Gate in the player's current Star
## System (ADR-0009). Re-run on Star System change to swap markers.
func _spawn_gate_markers() -> void:
	NavigationMarkerRenderer.spawn_gate_markers(_gates_root, _gates, WORLD_SCALE, _server_to_godot_pos)

## Spawn visual nodes for all celestial bodies in the current star system
## (stars + planets, ADR-0025). Re-called on system change.
func _spawn_body_markers() -> void:
	if _bodies_root == null:
		return
	_selected_body_id = -1
	NavigationMarkerRenderer.spawn_body_markers(_bodies_root, _bodies, WORLD_SCALE, _server_to_godot_pos)

## Update the sky shader's sun_direction each frame so the star appears in the
## correct direction relative to the player ship (ADR-0025).
func _update_sun_direction() -> void:
	if _sky_mat == null or _player_ship_id < 0 or not _ships.has(_player_ship_id):
		return
	var bodies: Array = _bodies
	var star_pos: Vector3 = Vector3.ZERO
	var found: bool = false
	for entry: Variant in bodies:
		var b: Dictionary = entry as Dictionary
		if (b.get("kind", "") as String) == "Star":
			star_pos = b.get("position", Vector3.ZERO) as Vector3
			found    = true
			break
	if not found:
		_sky_mat.set_shader_parameter("sun_active", 0.0)
		return

	## Player position in server units (undo WORLD_SCALE + Z inversion).
	var ship_node   : Node3D = _ships[_player_ship_id] as Node3D
	var ship_godot  : Vector3 = ship_node.global_position
	var ship_server : Vector3 = Vector3(ship_godot.x, ship_godot.y, -ship_godot.z) / WORLD_SCALE

	## Direction from ship toward star in server coords; map to Godot world space.
	var diff : Vector3 = star_pos - ship_server
	if diff.length_squared() < 1.0:
		_sky_mat.set_shader_parameter("sun_active", 0.0)
		return
	## Apply same coord mapping (Z inversion) so shader direction matches world.
	var godot_dir : Vector3 = Vector3(diff.x, diff.y, -diff.z).normalized()
	_sky_mat.set_shader_parameter("sun_direction", godot_dir)
	_sky_mat.set_shader_parameter("sun_active",    1.0)

	## Sun colour from the star's spectral type.
	var spec: float = 0.60
	for entry: Variant in bodies:
		var b: Dictionary = entry as Dictionary
		if (b.get("kind", "") as String) == "Star":
			spec = b.get("spectral_type", 0.60) as float
			break
	var sun_col: Color = NavigationMarkerRenderer.spectral_color(spec)
	_sky_mat.set_shader_parameter("sun_color", Vector3(sun_col.r, sun_col.g, sun_col.b))

## Tracks whether the player ship is within activation range of a Jump Gate
## (ADR-0009). Distance is computed in server units (Godot units / WORLD_SCALE).
func _update_gate_proximity() -> void:
	_nearby_gate_id = -1
	if _player_ship_id < 0 or not _ships.has(_player_ship_id):
		return
	var ship_pos: Vector3 = (_ships[_player_ship_id] as Node3D).global_position / WORLD_SCALE
	for gate: Variant in _gates:
		var g: Dictionary = gate as Dictionary
		var gate_pos: Vector3 = g.get("position", Vector3.ZERO) as Vector3
		if ship_pos.distance_to(gate_pos) <= (g.get("activation_radius", 0.0) as float):
			_nearby_gate_id = g.get("gate_id", -1) as int
			return

func _input(event: InputEvent) -> void:
	## Keyboard shortcuts: InputDecoder decides what the keypress means
	## (architecture-review-client.md C-1); this just performs the side
	## effects (network sends, warp-snap-pos / overlay state writes).
	if event is InputEventKey and event.pressed and not event.echo:
		var key: InputEventKey = event as InputEventKey
		var action: Dictionary = InputDecoder.decode_key(
			key.keycode, _player_ship_id,
			_selected_gate_id, _selected_target_id, _selected_body_id, _nearby_gate_id)
		match action.get("kind", "none") as String:
			"toggle_module":
				_toggle_module_by_index(action.module_index as int)
			"stop":
				_send_stop_command()
			"jump":
				var jump_gate: int = action.gate_id as int
				_connection.send_jump_command(_player_ship_id, jump_gate)
				if jump_gate != _nearby_gate_id:
					## Selected gate is out of range: server auto-warps first.
					_player_warp_snap_pos = _compute_warp_snap_pos(jump_gate)
			"approach_gate":
				_connection.send_approach_gate_command(_player_ship_id, action.gate_id as int)
			"approach_ship":
				_connection.send_approach_command(_player_ship_id, action.ship_id as int)
			"warp_to_gate":
				_connection.send_warp_command(_player_ship_id, action.gate_id as int)
				_player_warp_snap_pos = _compute_warp_snap_pos(action.gate_id as int)
			"warp_to_body":
				_connection.send_warp_to_body_command(_player_ship_id, action.body_id as int)
				_player_warp_snap_pos = _compute_body_warp_snap_pos(action.body_id as int)
			"toggle_tactical_overlay":
				if _tactical_overlay != null:
					(_tactical_overlay as Node3D).call("toggle_visible")
		return

	if event is InputEventMouseButton:
		var mb: InputEventMouseButton = event as InputEventMouseButton
		if mb.pressed:
			## A click on a module slot toggles it; it is never a world click.
			var slot_index: int = HudManager.module_slot_at(_module_slots, mb.position)
			if slot_index >= 0:
				if mb.button_index == MOUSE_BUTTON_LEFT:
					_toggle_module_by_index(slot_index)
				return
			match mb.button_index:
				MOUSE_BUTTON_LEFT:
					## Double-click steering takes priority and must work even when a
					## ship or gate is under the cursor (e.g. at spawn next to Gate 0).
					## Only a click that is NOT a double-click selects an approach target.
					if not _check_double_click(mb.position):
						var hit_ship: int = _pick_ship_at(mb.position)
						if hit_ship >= 0:
							_select_approach_target(hit_ship)
						else:
							var hit_gate: int = _pick_gate_at(mb.position)
							if hit_gate >= 0:
								_select_approach_gate(hit_gate)
							else:
								var hit_body: int = _pick_body_at(mb.position)
								if hit_body >= 0:
									_select_body(hit_body)
				MOUSE_BUTTON_RIGHT:
					## Right-click -> select lock-on target
					_try_lock_on(mb.position)

# -- Double-click detection ---------------------------------------------------

## Returns true when this click was consumed as a double-click (a move was
## issued, or suppressed only because the camera was dragging). The caller
## then skips approach-target selection so steering always wins.
func _check_double_click(pos: Vector2) -> bool:
	var now: float = Time.get_ticks_msec() / 1000.0
	var dt : float = now - _last_click_time
	var dp : float = pos.distance_to(_last_click_pos)

	if dt < DOUBLE_CLICK_SEC and dp < DOUBLE_CLICK_PX:
		## Ignore a double-click made while dragging the camera.
		var cam_dragging: bool = (_camera as Node).call("is_dragging") as bool
		if not cam_dragging:
			_on_double_click(pos)
		_last_click_time = -1.0  ## reset so a triple-click is not a 2nd double-click
		return true
	_last_click_time = now
	_last_click_pos  = pos
	return false

# -- Ship picking (screen position -> nearest ship ID) ------------------------
#
# Picking math lives in ship_picking.gd (ShipPicking, architecture-review-
# client.md C-1) -- main.gd only supplies the live candidate data.

## Returns the ship_id whose node is closest to the click ray (within
## ShipPicking.PICK_RADIUS_SHIP Godot units), excluding the player's own
## ship. -1 if nothing is hit.
func _pick_ship_at(screen_pos: Vector2) -> int:
	if _player_ship_id < 0:
		return -1
	return ShipPicking.pick_ship_at(_camera, screen_pos, _ships, _player_ship_id)

# -- Left-click -> select approach target (ADR-0015) -------------------------

## Select a ship as the Approach target. Press A to start approaching it.
func _select_approach_target(target_id: int) -> void:
	_selected_target_id = target_id
	_selected_gate_id   = -1
	_update_hud()

## Returns the gate_id of the Jump Gate (in the current system) whose marker
## is closest to the click ray, or -1. Gates are large objects, so the pick
## radius is wider than for ships.
func _pick_gate_at(screen_pos: Vector2) -> int:
	if _player_ship_id < 0:
		return -1
	return ShipPicking.pick_gate_at(_camera, screen_pos, _gates, _server_to_godot_pos)

## Select a Jump Gate as the Approach target. Press A to fly into its range.
func _select_approach_gate(gate_id: int) -> void:
	_selected_gate_id   = gate_id
	_selected_target_id = -1
	_selected_body_id   = -1
	_update_hud()

## Returns the body_id of the celestial body closest to the click ray, or -1.
func _pick_body_at(screen_pos: Vector2) -> int:
	if _player_ship_id < 0:
		return -1
	return ShipPicking.pick_body_at(_camera, screen_pos, _bodies_root, _bodies, WORLD_SCALE)

## Select a celestial body. Press W to warp to it.
func _select_body(body_id: int) -> void:
	_selected_body_id   = body_id
	_selected_gate_id   = -1
	_selected_target_id = -1
	_update_hud()

## Server-unit distance from the player ship to the selected gate, or -1 if
## there is no player ship or no selected gate (ADR-0022 warp gating / HUD).
func _selected_gate_distance() -> float:
	if _selected_gate_id < 0 or _player_ship_id < 0 or not _ships.has(_player_ship_id):
		return -1.0
	var ship_pos: Vector3 = (_ships[_player_ship_id] as Node3D).global_position / WORLD_SCALE
	for gate: Variant in _gates:
		var g: Dictionary = gate as Dictionary
		if (g.get("gate_id", -1) as int) != _selected_gate_id:
			continue
		var gpos: Vector3 = g.get("position", Vector3.ZERO) as Vector3
		## Godot Z is flipped; compute distance in server coordinate space.
		return Vector3(ship_pos.x, ship_pos.y, -ship_pos.z).distance_to(gpos)
	return -1.0

# -- Right-click -> LockOnCommand ---------------------------------------------

func _try_lock_on(screen_pos: Vector2) -> void:
	if _player_ship_id < 0:
		return

	var closest_id: int = _pick_ship_at(screen_pos)
	if closest_id >= 0:
		## Clear previous lock target
		if _player_lock_target >= 0 and _ships.has(_player_lock_target):
			(_ships[_player_lock_target] as Node3D).call("set_lock_state", "none")
		_player_lock_target = closest_id
		_connection.send_lock_on_command(_player_ship_id, closest_id)
		## Set Locking state and flash indicator
		if _ships.has(closest_id):
			(_ships[closest_id] as Node3D).call("set_lock_state", "locking")
			(_ships[closest_id] as Node3D).call("flash_lock_indicator")

# -- Double-click -> MoveCommand ----------------------------------------------

func _on_double_click(screen_pos: Vector2) -> void:
	if _player_ship_id < 0:
		return

	## Camera ray direction used directly as thrust direction (3D)
	var ray_dir: Vector3 = _camera.project_ray_normal(screen_pos)

	## Direction transform from Godot to server space (Z flip only; scale cancels on normalize)
	## Godot (x, y, -z) == server (x, y, z); for directions: (dx, dy, -dz)
	var server_dir: Vector3 = Vector3(ray_dir.x, ray_dir.y, -ray_dir.z)

	## Estimate player ship position in server space (back-calculated from lerped Godot position)
	var ship_godot_pos: Vector3 = Vector3.ZERO
	if _ships.has(_player_ship_id):
		ship_godot_pos = (_ships[_player_ship_id] as Node3D).global_position
	var ship_server_pos: Vector3 = Vector3(
		ship_godot_pos.x / WORLD_SCALE,
		ship_godot_pos.y / WORLD_SCALE,
		-ship_godot_pos.z / WORLD_SCALE,
	)

	## Set target far away so server treats normalize(target - ship) as server_dir
	var target: Vector3 = ship_server_pos + server_dir * 1_000_000.0
	_connection.send_move_command(_player_ship_id, target)

	## Show thrust arrow on player ship (ray_dir stays in Godot space)
	if _ships.has(_player_ship_id):
		(_ships[_player_ship_id] as Node3D).call("set_thrust_direction", ray_dir)

# -- S key -> StopCommand -----------------------------------------------------

func _send_stop_command() -> void:
	if _player_ship_id < 0:
		return
	_connection.send_stop_command(_player_ship_id)
	## Clear thrust arrow on player ship
	if _ships.has(_player_ship_id):
		(_ships[_player_ship_id] as Node3D).call("set_thrust_direction", Vector3.ZERO)

# -- Event handlers -----------------------------------------------------------

func _on_event_received(payload: Dictionary) -> void:
	_event_count += 1
	var event_type: String = payload.get("type", "") as String
	match event_type:
		"ShipSpawned"      : _handle_ship_spawned(payload)
		"VelocityChanged"  : _handle_velocity_changed(payload)
		"ShipDespawned"    : _handle_ship_despawned(payload)
		"DamageTaken"   : _handle_damage_taken(payload)
		"ShipDestroyed" : _handle_ship_destroyed(payload)
		"TargetLocked"  : _handle_target_locked(payload)
		"LockLost"      : _handle_lock_lost(payload)
		"JumpGateUsed"      : _handle_jump_gate_used(payload)
		"StarSystemChanged" : _handle_star_system_changed(payload)
		"AoiEnter"          : _handle_aoi_enter(payload)
		"AoiLeave"          : _handle_aoi_leave(payload)

# -- Jump Gate (ADR-0009) -----------------------------------------------------

## Ship passed through a Jump Gate -- teleport to entry_pos.
func _handle_jump_gate_used(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	if not _ships.has(ship_id):
		return
	var pos_dict: Dictionary = p.get("entry_pos", {}) as Dictionary
	var entry_pos := Vector3(
		pos_dict.get("x", 0.0) as float,
		pos_dict.get("y", 0.0) as float,
		pos_dict.get("z", 0.0) as float,
	)
	(_ships[ship_id] as Node3D).call("update_target", entry_pos)
	if ship_id == _player_ship_id:
		(_ships[ship_id] as Node3D).call("set_thrust_direction", Vector3.ZERO)
		_jump_notice       = "Jumped via Gate #%d" % (p.get("gate_id", 0) as int)
		_jump_notice_timer = 3.0

## Ship moved to a different star system -- show HUD notification.
func _handle_star_system_changed(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	var to_system: int = p.get("to_system", 0) as int
	var to_name: String = _system_names.get(to_system, "System %d" % to_system) as String
	if ship_id == _player_ship_id:
		_current_system_name = to_name
		_jump_notice         = "Entered %s system" % to_name
		_jump_notice_timer   = 3.0
		_selected_gate_id    = -1
		_selected_body_id    = -1
		## Gate / body markers refresh from the next InitialState (sent when the
		## client reconnects to the destination node after the Redirect).

func _on_connection_changed(connected: bool) -> void:
	if not connected:
		_clear_all_ships()

## Welcome received: just record player_id / ship_id.
## Ship nodes are spawned by the subsequent InitialState.
func _on_welcomed(_p_player_id: int, _p_ship_id: int) -> void:
	pass  ## connection.gd ship_id / player_id properties are already populated

## InitialState received: ingest the Sector's navigation map, then spawn all
## ship nodes in one pass. ShipSpawned events are not sent in Phase 5;
## InitialState handles initialization.
func _on_initial_state(state: Dictionary) -> void:
	_clear_all_ships()  ## Reset on reconnect
	HudManager.hide_duel_result(_duel_result_label)
	_ingest_star_map(state)

	for ship_data: Variant in (state.get("ships", []) as Array):
		_spawn_ship_from_data(ship_data as Dictionary)

## Store the server-provided navigation map (system names, this Sector's gates
## and bodies) and rebuild the gate / body markers from it. This replaces the
## previously hard-coded JUMP_GATES / CELESTIAL_BODIES / STAR_SYSTEM_NAMES.
func _ingest_star_map(state: Dictionary) -> void:
	_current_system_name = state.get("system_name", _current_system_name) as String

	_system_names.clear()
	for entry: Variant in (state.get("systems", []) as Array):
		var s: Dictionary = entry as Dictionary
		_system_names[s.get("id", -1) as int] = s.get("name", "") as String

	_gates.clear()
	for entry: Variant in (state.get("jump_gates", []) as Array):
		var g: Dictionary = entry as Dictionary
		var gp: Dictionary = g.get("position", {}) as Dictionary
		_gates.append({
			"gate_id"          : g.get("gate_id", -1) as int,
			"position"         : Vector3(gp.get("x", 0.0) as float, gp.get("y", 0.0) as float, gp.get("z", 0.0) as float),
			"activation_radius": g.get("activation_radius", 0.0) as float,
			"to_system_name"   : g.get("to_system_name", "") as String,
		})

	_bodies.clear()
	for entry: Variant in (state.get("celestial_bodies", []) as Array):
		var b: Dictionary = entry as Dictionary
		var bp: Dictionary = b.get("position", {}) as Dictionary
		_bodies.append({
			"body_id"      : b.get("id", -1) as int,
			"kind"         : b.get("kind", "") as String,
			"name"         : b.get("name", "") as String,
			"position"     : Vector3(bp.get("x", 0.0) as float, bp.get("y", 0.0) as float, bp.get("z", 0.0) as float),
			"radius"       : b.get("radius", 1.0) as float,
			"spectral_type": b.get("spectral_type", 0.0) as float,
		})

	_spawn_gate_markers()
	_spawn_body_markers()

## Materialize one ship node from a ship-state dict. Shared by InitialState and
## AoiEnter (ADR-0019). Skips ships already present.
func _spawn_ship_from_data(d: Dictionary) -> void:
	var sid      : int        = d.get("ship_id",   0)   as int
	if _ships.has(sid):
		return
	var is_player: bool       = d.get("is_player", false) as bool
	var pos_dict : Dictionary = d.get("position",  {})  as Dictionary
	var pos := Vector3(
		(pos_dict.get("x", 0.0) as float),
		(pos_dict.get("y", 0.0) as float),
		(pos_dict.get("z", 0.0) as float),
	)

	## Instantiate ship node
	var ship: Node3D = SHIP_SCENE.instantiate() as Node3D
	_ships_root.add_child(ship)
	ship.call("initialize", sid, pos)
	ship.name = "Ship_%d" % sid
	_ships[sid] = ship

	## Record HP (current + max) for every ship. The target panel needs each
	## ship's own maxima to render fill percentages.
	var msh: float = d.get("max_shield", 200.0) as float
	var mar: float = d.get("max_armor",  150.0) as float
	var mhu: float = d.get("max_hull",   150.0) as float
	var sh: float = d.get("current_shield", msh) as float
	var ar: float = d.get("current_armor",  mar) as float
	var hu: float = d.get("current_hull",   mhu) as float
	_ship_hp[sid] = {
		"shield": sh, "armor": ar, "hull": hu,
		"max_shield": msh, "max_armor": mar, "max_hull": mhu,
	}

	if sid == _connection.ship_id and _player_ship_id < 0:
		_player_max_shield = d.get("max_shield", 500.0) as float
		_player_max_armor  = d.get("max_armor",  300.0) as float
		_player_max_hull   = d.get("max_hull",   200.0) as float
		_player_shield     = sh
		_player_armor      = ar
		_player_hull       = hu
		## Initialize client-side capacitor simulation.
		_cap_max      = d.get("cap_max",               500.0) as float
		_cap_recharge = d.get("cap_recharge_per_tick",  10.0) as float
		_cap_current  = _cap_max  ## Assume full cap on connect.
		_player_ship_type_name = d.get("ship_type_name", "") as String
		_set_as_player_ship(sid, ship)
	elif is_player:
		## Other player ship = potential duel opponent
		if sid not in _opponent_ship_ids:
			_opponent_ship_ids.append(sid)

## AoI: a ship entered the player's neighborhood -- materialize it (ADR-0019).
func _handle_aoi_enter(p: Dictionary) -> void:
	var ship: Dictionary = p.get("ship", {}) as Dictionary
	if not ship.is_empty():
		_spawn_ship_from_data(ship)

## AoI: a ship left the player's neighborhood -- remove it locally with no death
## effect (it is still alive elsewhere, just out of view / ADR-0019).
func _handle_aoi_leave(p: Dictionary) -> void:
	var sid: int = p.get("ship_id", 0) as int
	if not _ships.has(sid):
		return
	(_ships[sid] as Node3D).queue_free()
	_ships.erase(sid)
	_ship_hp.erase(sid)
	if sid == _selected_target_id:
		_selected_target_id = -1

func _on_player_fitting(modules: Array) -> void:
	## Initialise cycle_remaining for client-side cap simulation.
	for m: Variant in modules:
		var mod_dict: Dictionary = m as Dictionary
		mod_dict["cycle_remaining"] = 0
		mod_dict["cap_forced_off"]  = false
	_player_modules = modules
	_module_slots = HudManager.rebuild_module_bar(_module_bar, _player_modules)
	_recalc_weapon_range()

func _recalc_weapon_range() -> void:
	## Sum weapon_range_add and falloff_range_add from active Weapon modules.
	var opt: float  = 0.0
	var fall: float = 0.0
	for m: Variant in _player_modules:
		var mod_dict: Dictionary = m as Dictionary
		if not (mod_dict.get("is_active", false) as bool):
			continue
		if mod_dict.get("kind", "") as String != "Weapon":
			continue
		var sd: Dictionary = mod_dict.get("stat_delta", {}) as Dictionary
		opt  += sd.get("weapon_range_add",  0.0) as float
		fall += sd.get("falloff_range_add", 0.0) as float
	_weapon_range   = opt
	_weapon_falloff = fall
	_update_tactical_overlay()

func _update_tactical_overlay() -> void:
	if _tactical_overlay == null:
		return
	## Convert server units -> Godot units before passing to the overlay.
	## weapon_range/_falloff are in server coordinate units; the overlay lives
	## in Godot world space which is scaled by WORLD_SCALE (0.1).
	_tactical_overlay.call("set_ranges",
		_weapon_range   * WORLD_SCALE,
		_weapon_falloff * WORLD_SCALE)

func _on_module_activated(p_ship_id: int, p_module_id: int, _slot: String) -> void:
	if p_ship_id != _player_ship_id:
		return
	for m: Variant in _player_modules:
		var mod_dict: Dictionary = m as Dictionary
		if mod_dict.get("module_id", 0) as int == p_module_id:
			mod_dict["is_active"]       = true
			mod_dict["cap_forced_off"]  = false
			mod_dict["cycle_remaining"] = 0  ## Next tick will start a fresh cycle.
			break
	_recalc_weapon_range()

func _on_module_deactivated(p_ship_id: int, p_module_id: int, _slot: String) -> void:
	if p_ship_id != _player_ship_id:
		return
	for m: Variant in _player_modules:
		var mod_dict: Dictionary = m as Dictionary
		if mod_dict.get("module_id", 0) as int == p_module_id:
			var was_active: bool = mod_dict.get("is_active", false) as bool
			mod_dict["is_active"]       = false
			mod_dict["cycle_remaining"] = 0
			## If it was ON, capacitor or player deactivated it.
			if was_active:
				mod_dict["cap_forced_off"] = true
			break
	_recalc_weapon_range()


func _toggle_module_by_index(f_index: int) -> void:
	if _player_ship_id < 0:
		return
	## F1-F8 map to active module indices 0-7 (High/Mid slots)
	var active_count: int = 0
	for m: Variant in _player_modules:
		var mod_dict: Dictionary = m as Dictionary
		if mod_dict.get("is_active_module", false) as bool == false:
			continue  ## Skip Passive modules
		if active_count == f_index:
			var mid : int    = mod_dict.get("module_id", 0)   as int
			var slot: String = mod_dict.get("slot",      "")  as String
			var currently_active: bool = mod_dict.get("is_active", false) as bool
			if currently_active:
				_connection.send_deactivate_module(_player_ship_id, mid, slot)
			else:
				_connection.send_activate_module(_player_ship_id, mid, slot)
			return
		active_count += 1

func _set_as_player_ship(p_ship_id: int, ship: Node3D) -> void:
	_player_ship_id = p_ship_id
	_apply_player_material(ship)
	ship.call("set_as_player")
	_camera.call("set_target", ship)
	## Attach tactical overlay to player ship so rings follow it automatically.
	if _tactical_overlay != null:
		_tactical_overlay.queue_free()
	var overlay_script: GDScript = load("res://scripts/tactical_overlay.gd") as GDScript
	if overlay_script != null:
		_tactical_overlay = Node3D.new()
		_tactical_overlay.set_script(overlay_script)
		ship.add_child(_tactical_overlay)
		_update_tactical_overlay()

# -- Domain event handlers ----------------------------------------------------

func _handle_ship_spawned(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	if _ships.has(ship_id):
		return

	var pos_dict: Dictionary = p.get("position", {}) as Dictionary
	var pos := Vector3(
		(pos_dict.get("x", 0.0) as float),
		(pos_dict.get("y", 0.0) as float),
		(pos_dict.get("z", 0.0) as float),
	)

	var ship: Node3D = SHIP_SCENE.instantiate() as Node3D
	_ships_root.add_child(ship)
	ship.call("initialize", ship_id, pos)
	ship.name = "Ship_%d" % ship_id
	_ships[ship_id] = ship

	## If this ship matches the player_id from Welcome, set it as the player ship
	if ship_id == _connection.ship_id and _player_ship_id < 0:
		_set_as_player_ship(ship_id, ship)

func _handle_velocity_changed(p: Dictionary) -> void:
	var ship_id : int = p.get("ship_id", 0) as int
	if not _ships.has(ship_id):
		return

	var vel_dict: Dictionary = p.get("velocity", {}) as Dictionary
	var server_vel := Vector3(
		(vel_dict.get("dx", 0.0) as float),
		(vel_dict.get("dy", 0.0) as float),
		(vel_dict.get("dz", 0.0) as float),
	)
	(_ships[ship_id] as Node3D).call("set_velocity", server_vel)

	## Detect warp arrival and snap position to correct dead-reckoning drift.
	## Warp speed (~5000 u/tick) accumulates large position errors; snap the
	## Godot node to the pre-computed arrival position (set at warp-start time).
	if ship_id == _player_ship_id and _player_warp_snap_pos != Vector3.INF:
		var speed: float = server_vel.length()
		if _player_was_warping and speed < 1.0:
			(_ships[ship_id] as Node3D).call("update_target", _player_warp_snap_pos)
			_player_warp_snap_pos = Vector3.INF
		_player_was_warping = speed >= WARP_SPEED_THRESHOLD

	var tick: int = p.get("tick", 0) as int
	if tick > _current_tick:
		var ticks_elapsed: int = tick - _current_tick
		_current_tick = tick
		_simulate_cap(ticks_elapsed)

## Shared core for gate/body warp arrival pre-computation. Uses the ship's
## actual position (known at command-send time, not drifted) to determine the
## approach direction, then places the snap point at `arrival_factor * radius`
## from the target's centre (ADR-0022/0025).
func _compute_warp_snap_pos_core(target_pos: Vector3, radius: float, arrival_factor: float) -> Vector3:
	if not _ships.has(_player_ship_id):
		return Vector3.INF
	var ship_node: Node3D  = _ships[_player_ship_id] as Node3D
	var gdot     : Vector3 = ship_node.global_position
	var ship_server_pos := Vector3(gdot.x / WORLD_SCALE, gdot.y / WORLD_SCALE, -gdot.z / WORLD_SCALE)
	var dir: Vector3 = ship_server_pos - target_pos
	dir = dir.normalized() if dir.length() > 0.001 else Vector3(-1.0, 0.0, 0.0)
	return target_pos + dir * radius * arrival_factor

## Pre-compute the warp arrival position in server coords for a Jump Gate.
## Arrives at GATE_WARP_ARRIVAL_FACTOR of the gate's activation radius —
## safely within jump range (activation_radius = 2000).
func _compute_warp_snap_pos(gate_id: int) -> Vector3:
	var target_gate: Dictionary = {}
	for g: Variant in _gates:
		if (g as Dictionary).get("gate_id", -1) as int == gate_id:
			target_gate = g as Dictionary
			break
	if target_gate.is_empty():
		return Vector3.INF
	var gate_pos    : Vector3 = target_gate.get("position", Vector3.ZERO) as Vector3
	var activation_r: float   = target_gate.get("activation_radius", 2000.0) as float
	return _compute_warp_snap_pos_core(gate_pos, activation_r, GATE_WARP_ARRIVAL_FACTOR)

## Pre-compute the warp arrival position for a celestial body in server coords.
## Arrives at BODY_WARP_ARRIVAL_FACTOR of the body's radius from its centre
## (ADR-0025).
func _compute_body_warp_snap_pos(body_id: int) -> Vector3:
	var body_pos   : Vector3 = Vector3.ZERO
	var body_radius: float   = 1.0
	var found: bool          = false
	for entry: Variant in _bodies:
		var b: Dictionary = entry as Dictionary
		if (b.get("body_id", -1) as int) == body_id:
			body_pos    = b.get("position", Vector3.ZERO) as Vector3
			body_radius = b.get("radius", 1.0) as float
			found = true
			break
	if not found:
		return Vector3.INF
	return _compute_warp_snap_pos_core(body_pos, body_radius, BODY_WARP_ARRIVAL_FACTOR)

func _handle_ship_despawned(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	if not _ships.has(ship_id):
		return
	var ship: Node3D = _ships[ship_id] as Node3D
	ship.queue_free()
	_ships.erase(ship_id)
	if ship_id == _selected_target_id:
		_selected_target_id = -1
	if ship_id == _player_ship_id:
		_player_ship_id = -1

func _handle_damage_taken(p: Dictionary) -> void:
	var ship_id: int   = p.get("ship_id",        0)   as int
	var sh     : float = p.get("current_shield", 0.0) as float
	var ar     : float = p.get("current_armor",  0.0) as float
	var hu     : float = p.get("current_hull",   0.0) as float
	## Update current HP in place so the maxima recorded at spawn survive.
	var entry: Dictionary = _ship_hp.get(ship_id, {}) as Dictionary
	entry["shield"] = sh
	entry["armor"]  = ar
	entry["hull"]   = hu
	_ship_hp[ship_id] = entry
	if ship_id == _player_ship_id:
		_player_shield = sh
		_player_armor  = ar
		_player_hull   = hu
	## Flash red on any ship that takes damage (visual hit feedback)
	if _ships.has(ship_id):
		(_ships[ship_id] as Node3D).call("flash_damage")

func _handle_ship_destroyed(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	if not _ships.has(ship_id):
		return
	var ship: Node3D = _ships[ship_id] as Node3D
	_ships.erase(ship_id)
	_ship_hp.erase(ship_id)
	if ship_id == _selected_target_id:
		_selected_target_id = -1
	## Play destruction effect (queue_free happens inside play_destroy_effect)
	ship.call("play_destroy_effect")
	if ship_id == _player_ship_id:
		_player_ship_id     = -1
		_player_shield      = 0.0
		_player_armor       = 0.0
		_player_hull        = 0.0
		_player_lock_target = -1
		HudManager.show_duel_result(_duel_result_label, false)  ## DEFEAT
	elif ship_id in _opponent_ship_ids:
		_opponent_ship_ids.erase(ship_id)
		HudManager.show_duel_result(_duel_result_label, true)   ## VICTORY
	## Clear lock target if it was just destroyed
	if ship_id == _player_lock_target:
		_player_lock_target = -1

func _handle_target_locked(p: Dictionary) -> void:
	var locker_id: int = p.get("locker_id", 0) as int
	var target_id: int = p.get("target_id", 0) as int
	## Player completed a lock
	if locker_id == _player_ship_id:
		_player_lock_target = target_id
		if _ships.has(target_id):
			(_ships[target_id] as Node3D).call("set_lock_state", "locked")
	## Locked by another ship (no visual indicator)

func _handle_lock_lost(p: Dictionary) -> void:
	var locker_id: int = p.get("locker_id", 0) as int
	var target_id: int = p.get("target_id", 0) as int
	if locker_id == _player_ship_id:
		if _ships.has(target_id):
			(_ships[target_id] as Node3D).call("set_lock_state", "none")
		if target_id == _player_lock_target:
			_player_lock_target = -1

# -- HUD ----------------------------------------------------------------------

func _update_hud() -> void:
	var speed_str: String = "-"
	if _player_ship_id >= 0 and _ships.has(_player_ship_id):
		var spd: float = (_ships[_player_ship_id] as Node3D).call("get_speed_server") as float
		speed_str = "%d m/s" % int(spd * METERS_PER_UNIT)
	HudManager.update_status_panel(
		_status_panel_refs, _connection.is_connected_to_server(),
		_player_ship_type_name, _current_system_name, speed_str)

	HudManager.update_ship_status_panel(
		_ship_status_refs, _player_ship_id,
		_player_shield, _player_max_shield, _player_armor, _player_max_armor, _player_hull, _player_max_hull,
		_cap_current, _cap_max)

	var target_known: bool = _ships.has(_player_lock_target)
	var dist_text: String = "—"
	if target_known and _player_ship_id >= 0 and _ships.has(_player_ship_id):
		var dist_m: float = (_ships[_player_ship_id] as Node3D).global_position.distance_to(
			(_ships[_player_lock_target] as Node3D).global_position) / WORLD_SCALE
		dist_text = "%.1f km" % (dist_m * METERS_PER_UNIT / 1000.0)
	var target_hp: Dictionary = _ship_hp.get(_player_lock_target, {}) as Dictionary
	HudManager.update_target_panel(_target_panel_refs, _player_lock_target, target_known, dist_text, target_hp)

	HudManager.update_module_bar(_module_slots, _player_modules)

	var jump_line  : String = ""
	if _nearby_gate_id >= 0:
		jump_line = "\n[J] Jump Gate #%d" % _nearby_gate_id
	if _jump_notice != "":
		jump_line += "\n" + _jump_notice

	## Approach / warp target selection (ADR-0015 / ADR-0022 / ADR-0025).
	var approach_line: String = ""
	if _selected_gate_id >= 0:
		approach_line = "\n[A] Approach Gate #%d" % _selected_gate_id
		## Warp is only valid beyond the minimum warp distance (ADR-0022).
		var gate_dist: float = _selected_gate_distance()
		if gate_dist >= MIN_WARP_DISTANCE:
			approach_line += "\n[W] Warp  [J] Warp+Jump"
		elif gate_dist >= 0.0:
			approach_line += "\n[W] too close to warp"
	elif _selected_body_id >= 0:
		## Look up body name for HUD.
		var body_name: String = "Body #%d" % _selected_body_id
		for entry: Variant in _bodies:
			var b: Dictionary = entry as Dictionary
			if (b.get("body_id", -1) as int) == _selected_body_id:
				body_name = b.get("name", body_name) as String
				break
		approach_line = "\n[W] Warp to %s" % body_name
	elif _selected_target_id >= 0:
		approach_line = "\n[A] Approach #%d" % _selected_target_id

	_stats_label.text = (
		"Ships: %d\nTick: %d%s\n\n[Click] Select  [DoubleClick] Thrust\n[RightClick] Lock%s"
		% [_ships.size(), _current_tick, approach_line, jump_line]
	)

# -- Capacitor client-side simulation -----------------------------------------

## Mirror of CapacitorSystem::run() -- called once per tick elapsed.
## Keeps cap display in sync without any extra server messages.
func _simulate_cap(ticks: int) -> void:
	if _cap_current < 0.0 or _player_ship_id < 0:
		return

	for _t: int in range(ticks):
		## Recharge.
		_cap_current = minf(_cap_current + _cap_recharge, _cap_max)

		## Cycle logic for each active module.
		for m: Variant in _player_modules:
			var mod: Dictionary = m as Dictionary
			if not (mod.get("is_active_module", false) as bool):
				continue
			if not (mod.get("is_active", false) as bool):
				continue

			var cycle_rem: int   = mod.get("cycle_remaining", 0) as int
			var cost     : float = mod.get("cap_cost_per_cycle", 0.0) as float
			var cycle_t  : int   = mod.get("cycle_time_ticks",  10)  as int

			if cycle_rem == 0:
				## Try to start new cycle.
				if cost <= 0.0 or _cap_current >= cost:
					_cap_current -= cost
					mod["cycle_remaining"] = cycle_t
				## If not enough cap, server will emit ModuleDeactivated -- skip here.
			else:
				mod["cycle_remaining"] = cycle_rem - 1

		_cap_current = maxf(_cap_current, 0.0)

# -- Internal utilities -------------------------------------------------------

func _clear_all_ships() -> void:
	for ship_node: Node3D in _ships.values():
		if is_instance_valid(ship_node):
			ship_node.queue_free()
	_ships.clear()
	_ship_hp.clear()
	_opponent_ship_ids.clear()
	_player_ship_id     = -1
	_player_shield      = -1.0
	_player_armor       = -1.0
	_player_hull        = -1.0
	_player_lock_target = -1
	_selected_target_id = -1
	_selected_gate_id   = -1
	_selected_body_id   = -1
	_current_tick       = 0
	_event_count        = 0

func _setup_space_environment() -> void:
	## Build the procedural space sky at runtime.
	## WorldEnvironment is created dynamically -- no .tscn changes needed.
	var shader := load("res://shaders/space_sky.gdshader") as Shader
	if shader == null:
		push_warning("[Main] space_sky.gdshader not found")
		return

	var sky_mat := ShaderMaterial.new()
	sky_mat.shader = shader
	_sky_mat = sky_mat

	## Tweak nebula / star appearance here without editing the shader.
	sky_mat.set_shader_parameter("star_threshold",    0.960)
	sky_mat.set_shader_parameter("star_brightness",   3.5)
	sky_mat.set_shader_parameter("nebula_strength",   0.40)
	sky_mat.set_shader_parameter("milkyway_strength", 0.12)
	sky_mat.set_shader_parameter("milkyway_color",    Color(0.48, 0.58, 0.90))
	sky_mat.set_shader_parameter("ambient_color",     Color(0.004, 0.003, 0.010))

	var sky := Sky.new()
	sky.sky_material  = sky_mat
	sky.process_mode  = Sky.PROCESS_MODE_REALTIME
	sky.radiance_size = Sky.RADIANCE_SIZE_256

	var env := Environment.new()
	env.background_mode      = Environment.BG_SKY
	env.sky                  = sky
	env.ambient_light_source = Environment.AMBIENT_SOURCE_SKY
	env.ambient_light_energy = 0.03   ## Space is very dark
	env.tonemap_mode         = Environment.TONE_MAPPER_FILMIC
	env.tonemap_exposure     = 1.0
	env.tonemap_white        = 6.0    ## Prevent star bloom clipping

	## Bloom -- makes ship emissions and bright stars glow cinematically.
	env.glow_enabled       = true
	env.glow_normalized    = false
	env.glow_intensity     = 0.8
	env.glow_bloom         = 0.10
	env.glow_blend_mode    = Environment.GLOW_BLEND_MODE_SOFTLIGHT
	## Stars peak at ~1.5; engine glow emission is x12.
	## Set threshold above star peak so only ship emissions trigger bloom.
	env.glow_hdr_threshold = 2.0
	env.glow_hdr_scale     = 1.0

	var world_env         := WorldEnvironment.new()
	world_env.environment  = env
	add_child(world_env)

func _build_player_material() -> void:
	_player_material = StandardMaterial3D.new()
	_player_material.albedo_color               = Color(1.0, 0.5, 0.1, 1)
	_player_material.metallic                   = 0.9
	_player_material.roughness                  = 0.2
	_player_material.emission_enabled           = true
	_player_material.emission                   = Color(1.0, 0.3, 0.0, 1)
	_player_material.emission_energy_multiplier = 1.5

func _apply_player_material(ship: Node3D) -> void:
	var hull: MeshInstance3D = ship.get_node_or_null("Hull") as MeshInstance3D
	if hull != null:
		hull.set_surface_override_material(0, _player_material)

